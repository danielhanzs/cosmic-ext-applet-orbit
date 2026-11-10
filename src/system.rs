// SPDX-License-Identifier: GPL-3.0-only

//! How busy the machine is: the processor, the memory, the network, read
//! from /proc. Counters come as totals; the rates are two reads apart.

use std::time::Instant;

#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub at: Instant,
    /// Processor time busy, and in all, since boot.
    cpu: (u64, u64),
    /// Memory in use and in all, in kB.
    pub memory: (u64, u64),
    /// Bytes received and sent, since boot, on the real interfaces.
    network: (u64, u64),
}

/// What changed between two samples.
#[derive(Debug, Clone, Copy, Default)]
pub struct Load {
    /// 0..1
    pub cpu: f32,
    /// Bytes a second, down and up.
    pub down: f64,
    pub up: f64,
}

pub async fn sample() -> Option<Sample> {
    let stat = tokio::fs::read_to_string("/proc/stat").await.ok()?;
    let meminfo = tokio::fs::read_to_string("/proc/meminfo").await.ok()?;
    // The sandbox's own network is only loopback: the host's counters.
    let net = crate::host::command("cat").arg("/proc/net/dev").output().await.ok()?;
    Some(Sample {
        at: Instant::now(),
        cpu: cpu(&stat)?,
        memory: memory(&meminfo)?,
        network: network(&String::from_utf8_lossy(&net.stdout)),
    })
}

impl Sample {
    pub fn since(&self, before: &Sample) -> Load {
        let (busy, all) = (self.cpu.0.saturating_sub(before.cpu.0), self.cpu.1.saturating_sub(before.cpu.1));
        let secs = self.at.duration_since(before.at).as_secs_f64().max(0.1);
        Load {
            cpu: if all == 0 { 0.0 } else { busy as f32 / all as f32 },
            down: self.network.0.saturating_sub(before.network.0) as f64 / secs,
            up: self.network.1.saturating_sub(before.network.1) as f64 / secs,
        }
    }
}

/// "cpu  user nice system idle iowait irq softirq steal …"
fn cpu(stat: &str) -> Option<(u64, u64)> {
    let fields: Vec<u64> = stat.lines().next()?.split_whitespace().skip(1).filter_map(|f| f.parse().ok()).collect();
    let all: u64 = fields.iter().take(8).sum();
    let idle = fields.get(3)? + fields.get(4).unwrap_or(&0);
    Some((all - idle, all))
}

fn memory(meminfo: &str) -> Option<(u64, u64)> {
    let field = |name: &str| {
        meminfo.lines().find_map(|l| l.strip_prefix(name)?.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
    };
    let total = field("MemTotal:")?;
    Some((total.saturating_sub(field("MemAvailable:")?), total))
}

/// Summed over the interfaces, but loopback and the virtual ones that
/// would count the same bytes twice.
fn network(dev: &str) -> (u64, u64) {
    dev.lines()
        .skip(2)
        .filter_map(|line| {
            let (name, counters) = line.split_once(':')?;
            let name = name.trim();
            if name == "lo" || ["veth", "docker", "br-", "virbr", "tun", "tap", "wg"].iter().any(|p| name.starts_with(p)) {
                return None;
            }
            let fields: Vec<u64> = counters.split_whitespace().filter_map(|f| f.parse().ok()).collect();
            Some((*fields.first()?, *fields.get(8)?))
        })
        .fold((0, 0), |(rx, tx), (r, t)| (rx + r, tx + t))
}

/// "1.2 MB/s", "340 kB/s"
pub fn rate(bytes: f64) -> String {
    match bytes {
        b if b >= 1e6 => format!("{:.1} MB/s", b / 1e6),
        b if b >= 1e3 => format!("{:.0} kB/s", b / 1e3),
        b => format!("{b:.0} B/s"),
    }
}

/// "5.2 / 15.6 GB"
pub fn gigabytes((used, total): (u64, u64)) -> String {
    format!("{:.1} / {:.1} GB", used as f64 / 1048576.0, total as f64 / 1048576.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_proc() {
        assert_eq!(cpu("cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 1 2 3"), Some((150, 1000)));
        assert_eq!(memory("MemTotal:       16000000 kB\nMemFree: 1 kB\nMemAvailable:   10000000 kB\n"), Some((6000000, 16000000)));
        let dev = "Inter-|   Receive\n face |bytes\n    lo: 500 1 0 0 0 0 0 0 500 1 0 0 0 0 0 0\n\
                   wlp2s0: 1000 10 0 0 0 0 0 0 200 5 0 0 0 0 0 0\n docker0: 9 9 0 0 0 0 0 0 9 9 0 0 0 0 0 0\n";
        assert_eq!(network(dev), (1000, 200));
        assert_eq!(rate(1_234_567.0), "1.2 MB/s");
        assert_eq!(rate(340_000.0), "340 kB/s");
    }
}
