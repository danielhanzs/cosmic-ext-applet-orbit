// SPDX-License-Identifier: GPL-3.0-only

//! For developers: SSH hosts to connect to, the ports something listens
//! on, the busiest processes, and libvirt's virtual machines. All through
//! the host's own tools (ssh, ss, ps, kill, virsh), from a Flatpak too.


async fn host(program: &str, args: &[&str]) -> Option<String> {
    crate::host::output(program, args, 20).await.ok()
}

/// Something a terminal may be asked to run with: not an option, no spaces.
pub fn safe_word(word: &str) -> bool {
    !word.is_empty() && !word.starts_with('-') && !word.chars().any(|c| c.is_whitespace() || c.is_control())
}

// ---------------------------------------------------------------- SSH

#[derive(Debug, Clone, PartialEq)]
pub struct SshHost {
    /// Its name in the config: what `ssh` is given.
    pub name: String,
    /// "user@hostname", as far as the config says.
    pub target: String,
}

/// The hosts of ~/.ssh/config and the files it includes; patterns left out.
pub async fn ssh_hosts() -> Vec<SshHost> {
    // Read on the host: a Flatpak doesn't see ~/.ssh.
    let script = r#"cd "$HOME/.ssh" 2>/dev/null || exit 0
cat config 2>/dev/null
for pattern in $(sed -n 's/^[[:space:]]*[Ii]nclude[[:space:]]\+//p' config 2>/dev/null); do
  case $pattern in "~/"*) pattern="$HOME/${pattern#\~/}";; esac
  for file in $pattern; do [ -f "$file" ] && cat "$file"; done
done"#;
    host("sh", &["-c", script]).await.map(|text| parse_ssh(&text)).unwrap_or_default()
}

fn parse_ssh(text: &str) -> Vec<SshHost> {
    let mut hosts: Vec<SshHost> = Vec::new();
    // The hosts of the block being read, and what it says of them.
    let (mut names, mut hostname, mut user): (Vec<String>, String, String) = (Vec::new(), String::new(), String::new());
    let mut flush = |names: &mut Vec<String>, hostname: &str, user: &str| {
        for name in names.drain(..) {
            let host = if hostname.is_empty() { name.clone() } else { hostname.to_string() };
            let target = if user.is_empty() { host } else { format!("{user}@{host}") };
            if !hosts.iter().any(|h| h.name == name) {
                hosts.push(SshHost { name, target });
            }
        }
    };
    for line in text.lines().map(str::trim) {
        let Some((key, value)) = line.split_once(|c: char| c.is_whitespace() || c == '=') else { continue };
        let value = value.trim_start_matches(|c: char| c.is_whitespace() || c == '=').trim();
        match key.to_lowercase().as_str() {
            "host" | "match" => {
                flush(&mut names, &hostname, &user);
                hostname.clear();
                user.clear();
                if key.eq_ignore_ascii_case("host") {
                    names = value.split_whitespace().filter(|n| !n.contains(['*', '?', '!']) && safe_word(n)).map(String::from).collect();
                }
            }
            "hostname" => hostname = value.to_string(),
            "user" => user = value.to_string(),
            _ => {}
        }
    }
    flush(&mut names, &hostname, &user);
    hosts
}

/// `ssh <destination>` in COSMIC's terminal.
pub fn ssh(destination: &str) {
    if safe_word(destination) {
        crate::host::spawn("cosmic-term", &["-e", "ssh", destination]);
    }
}

// ---------------------------------------------------------------- ports

#[derive(Debug, Clone, PartialEq)]
pub struct Port {
    pub port: u16,
    /// Where: "127.0.0.1", "0.0.0.0", "[::]"…
    pub address: String,
    /// Who listens, where it's one of the user's (others aren't told).
    pub process: Option<(String, u32)>,
}

/// TCP ports something listens on, each once.
pub async fn ports() -> Vec<Port> {
    host("ss", &["-ltnpH"]).await.map(|text| parse_ss(&text)).unwrap_or_default()
}

/// `ss -ltnpH`: "LISTEN 0 4096 127.0.0.1:631 0.0.0.0:* users:(("cupsd",pid=123,fd=7))"
fn parse_ss(text: &str) -> Vec<Port> {
    let mut ports: Vec<Port> = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let Some(local) = fields.get(3) else { continue };
        let Some((address, port)) = local.rsplit_once(':') else { continue };
        let Ok(port) = port.parse::<u16>() else { continue };
        let process = line.split_once("users:((\"").and_then(|(_, rest)| {
            let (name, rest) = rest.split_once('"')?;
            let pid = rest.split_once("pid=")?.1.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
            Some((name.to_string(), pid))
        });
        // The same port on IPv4 and IPv6 is one.
        if let Some(seen) = ports.iter_mut().find(|p| p.port == port && (p.process.is_none() || process.is_none() || p.process == process)) {
            if seen.process.is_none() {
                seen.process = process;
            }
            continue;
        }
        // "127.0.0.53%lo" → "127.0.0.53"; "[::ffff:127.0.0.1]" → "127.0.0.1".
        let address = address.split('%').next().unwrap_or(address);
        let address = address.strip_prefix("[::ffff:").and_then(|a| a.strip_suffix(']')).unwrap_or(address);
        ports.push(Port { port, address: address.to_string(), process });
    }
    ports.sort_by_key(|p| p.port);
    ports
}

/// Well-known ports, the most used first, and what they're for.
pub const KNOWN: [(u16, &str); 12] = [
    (21, "FTP"),
    (22, "SSH"),
    (80, "HTTP"),
    (443, "HTTPS"),
    (8080, "HTTP"),
    (3000, "HTTP"),
    (8000, "HTTP"),
    (5173, "Vite"),
    (3306, "MySQL"),
    (5432, "PostgreSQL"),
    (6379, "Redis"),
    (27017, "MongoDB"),
];

/// Ports matching `query` (a number's start, or a process's name); without
/// one, the well-known ones that listen, five at most.
pub fn find_ports<'a>(ports: &'a [Port], query: &str) -> Vec<&'a Port> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return KNOWN.iter().filter_map(|(n, _)| ports.iter().find(|p| p.port == *n)).take(5).collect();
    }
    ports
        .iter()
        .filter(|p| {
            p.port.to_string().starts_with(&query) || p.process.as_ref().is_some_and(|(name, _)| name.to_lowercase().contains(&query))
        })
        .take(10)
        .collect()
}

// ---------------------------------------------------------------- processes

#[derive(Debug, Clone, PartialEq)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    /// Percent of one core.
    pub cpu: f32,
    /// Resident memory, kB.
    pub memory: u64,
    /// The user's own: one they may end.
    pub own: bool,
}

/// The five busiest, by processor (or memory) use.
pub async fn processes(by_memory: bool) -> Vec<Process> {
    let sort = if by_memory { "--sort=-rss" } else { "--sort=-pcpu" };
    let me = std::env::var("USER").unwrap_or_default();
    host("ps", &["-eo", "pid=,user:64=,pcpu=,rss=,comm=", sort])
        .await
        .map(|text| parse_ps(&text, &me).into_iter().take(5).collect())
        .unwrap_or_default()
}

fn parse_ps(text: &str, me: &str) -> Vec<Process> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let pid = f.next()?.parse().ok()?;
            let user = f.next()?;
            let cpu = f.next()?.parse().ok()?;
            let memory = f.next()?.parse().ok()?;
            let name = f.collect::<Vec<_>>().join(" ");
            Some(Process { pid, name, cpu, memory, own: user == me })
        })
        .collect()
}

/// Asks a process to end (SIGTERM): one of the user's, as `kill` lets.
pub async fn end(pid: u32) {
    host("kill", &[&pid.to_string()]).await;
}

// ---------------------------------------------------------------- virtual machines

const LIBVIRT: &str = "qemu:///system";

#[derive(Debug, Clone, PartialEq)]
pub struct Vm {
    pub name: String,
    /// "running", "shut off", "paused"…
    pub state: String,
}

impl Vm {
    pub fn running(&self) -> bool {
        self.state == "running"
    }
}

/// libvirt's machines; None: no libvirt to ask.
pub async fn vms() -> Option<Vec<Vm>> {
    host("virsh", &["-c", LIBVIRT, "list", "--all"]).await.map(|text| parse_virsh(&text))
}

/// " Id   Name          State\n----\n 1    win11   running\n -    lab   shut off"
fn parse_virsh(text: &str) -> Vec<Vm> {
    text.lines()
        .skip_while(|l| !l.starts_with("--"))
        .skip(1)
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let _id = f.next()?;
            let name = f.next()?.to_string();
            let state = f.collect::<Vec<_>>().join(" ");
            safe_word(&name).then_some(Vm { name, state })
        })
        .collect()
}

/// "start" or "shutdown" (asks the guest to shut down).
pub async fn vm(action: &'static str, name: String) {
    host("virsh", &["-c", LIBVIRT, action, "--domain", &name]).await;
}

/// Its screen, in virt-viewer.
pub fn console(name: &str) {
    if safe_word(name) {
        crate::host::spawn("virt-viewer", &["--connect", LIBVIRT, "--wait", name]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ssh_config() {
        let config = "Host *\n  ServerAliveInterval 30\nHost web web2\n  HostName example.com\n  User deploy\n\
                      Host db\n  HostName=10.0.0.5\nHost -oProxyCommand=x\nMatch all\n  User x\n";
        let hosts = parse_ssh(config);
        let got: Vec<(&str, &str)> = hosts.iter().map(|h| (h.name.as_str(), h.target.as_str())).collect();
        assert_eq!(got, [("web", "deploy@example.com"), ("web2", "deploy@example.com"), ("db", "10.0.0.5")]);
        assert!(safe_word("me@host.example") && !safe_word("-oProxyCommand=x") && !safe_word("a b"));
    }

    #[test]
    fn reads_ss_ps_virsh() {
        let ss = "LISTEN 0 4096 127.0.0.1:631 0.0.0.0:* users:((\"cupsd\",pid=123,fd=7))\n\
                  LISTEN 0 4096 0.0.0.0:8080 0.0.0.0:*\nLISTEN 0 4096 [::]:8080 [::]:*\n\
                  LISTEN 0 4096 127.0.0.53%lo:53 0.0.0.0:*\n\
                  LISTEN 0 50 [::ffff:127.0.0.1]:24166 *:* users:((\"ide\",pid=9,fd=1))\n\
                  LISTEN 0 50 [::ffff:127.0.0.1]:41109 *:*\n";
        let ports = parse_ss(ss);
        assert_eq!(ports.iter().map(|p| p.port).collect::<Vec<_>>(), [53, 631, 8080, 24166, 41109]);
        assert_eq!(ports[3].address, "127.0.0.1");
        let found: Vec<u16> = find_ports(&ports, "80").iter().map(|p| p.port).collect();
        assert_eq!(found, [8080]);
        assert_eq!(find_ports(&ports, "cups")[0].port, 631);
        // Nothing asked: the well-known ones that listen.
        assert_eq!(find_ports(&ports, "").iter().map(|p| p.port).collect::<Vec<_>>(), [8080]);
        assert_eq!(ports[0].address, "127.0.0.53");
        assert_eq!(ports[1].process, Some(("cupsd".into(), 123)));
        let ps = "  42 me  95.4 357680 php-fpm: pool\n 7 root 1.0 100 sshd\n";
        let p = parse_ps(ps, "me");
        assert_eq!((p[0].pid, p[0].name.as_str(), p[0].own, p[1].own), (42, "php-fpm: pool", true, false));
        let virsh = " Id   Name          State\n------------------------------\n 3    win11   running\n -    lab-vm   shut off\n\n";
        let v = parse_virsh(virsh);
        assert_eq!(v, [Vm { name: "win11".into(), state: "running".into() }, Vm { name: "lab-vm".into(), state: "shut off".into() }]);
    }
}
