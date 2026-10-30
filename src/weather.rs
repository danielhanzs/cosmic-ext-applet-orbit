// SPDX-License-Identifier: GPL-3.0-only

//! The weather now and the days ahead, from Open-Meteo: free, without an
//! account. The place is the one set, or the time zone's city.

use jiff::civil;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct Weather {
    pub place: String,
    pub temperature: f64,
    pub feels_like: f64,
    pub wind: f64,
    pub condition: Condition,
    pub day: bool,
    pub days: Vec<Day>,
    pub fahrenheit: bool,
}

/// A place found by name, kept with where it is.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Place {
    pub name: String,
    /// Its region and country, to tell same-named places apart.
    pub region: String,
    pub latitude: f64,
    pub longitude: f64,
}

impl Place {
    pub fn label(&self) -> String {
        if self.region.is_empty() { self.name.clone() } else { format!("{}, {}", self.name, self.region) }
    }
}

#[derive(Debug, Clone)]
pub struct Day {
    pub date: civil::Date,
    pub condition: Condition,
    pub high: f64,
    pub low: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    Clear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Storm,
}

impl Condition {
    /// From a WMO weather code.
    pub fn from_code(code: u64) -> Self {
        match code {
            0 => Condition::Clear,
            1 | 2 => Condition::PartlyCloudy,
            3 => Condition::Cloudy,
            45 | 48 => Condition::Fog,
            51..=57 => Condition::Drizzle,
            71..=77 | 85 | 86 => Condition::Snow,
            95..=99 => Condition::Storm,
            _ => Condition::Rain,
        }
    }

    pub fn icon(self, day: bool) -> &'static str {
        match (self, day) {
            (Condition::Clear, true) => "weather-clear-symbolic",
            (Condition::Clear, false) => "weather-clear-night-symbolic",
            (Condition::PartlyCloudy, true) => "weather-few-clouds-symbolic",
            (Condition::PartlyCloudy, false) => "weather-few-clouds-night-symbolic",
            (Condition::Cloudy, _) => "weather-overcast-symbolic",
            (Condition::Fog, _) => "weather-fog-symbolic",
            (Condition::Drizzle, _) => "weather-showers-scattered-symbolic",
            (Condition::Rain, _) => "weather-showers-symbolic",
            (Condition::Snow, _) => "weather-snow-symbolic",
            (Condition::Storm, _) => "weather-storm-symbolic",
        }
    }
}

/// A small JSON document over HTTPS, through the host's curl.
async fn fetch(url: &str) -> Option<Value> {
    let out = crate::host::command("curl")
        .args(["-sf", "--globoff", "--proto", "=https", "--max-time", "10", "--max-filesize", "1000000"])
        .arg(url)
        .output()
        .await
        .ok()?;
    out.status.success().then(|| serde_json::from_slice(&out.stdout).ok())?
}

/// "Europe/Budapest" → "Budapest"; "America/New_York" → "New York".
fn time_zone_city() -> Option<String> {
    let tz = jiff::tz::TimeZone::system();
    let name = tz.iana_name()?;
    let city = name.rsplit('/').next()?.replace('_', " ");
    (!city.is_empty() && name.contains('/')).then_some(city)
}

fn encoded(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Places matching a name, for picking one. `language`: for their names.
pub async fn search(query: String, language: String) -> Vec<Place> {
    let Some(found) = fetch(&format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=6&language={}&format=json",
        encoded(query.trim()),
        encoded(&language),
    ))
    .await
    else {
        return Vec::new();
    };
    found["results"]
        .as_array()
        .map(|results| {
            results
                .iter()
                .filter_map(|r| {
                    let mut region: Vec<&str> =
                        ["admin1", "country"].iter().filter_map(|k| r[*k].as_str()).collect();
                    let name = r["name"].as_str()?;
                    region.retain(|part| *part != name);
                    region.dedup();
                    Some(Place {
                        name: name.to_string(),
                        region: region.join(", "),
                        latitude: r["latitude"].as_f64()?,
                        longitude: r["longitude"].as_f64()?,
                    })
                })
                .fold(Vec::<Place>::new(), |mut places, place| {
                    // The same name and region twice (a city and its district) is one choice.
                    if !places.iter().any(|p| p.label() == place.label()) {
                        places.push(place);
                    }
                    places
                })
        })
        .unwrap_or_default()
}

/// The weather at `place`, or, without one, at the time zone's city.
pub async fn read(place: Option<Place>, language: String, fahrenheit: bool) -> Option<Weather> {
    let place = match place {
        Some(place) => place,
        None => search(time_zone_city()?, language).await.into_iter().next()?,
    };
    let units = if fahrenheit { "&temperature_unit=fahrenheit&wind_speed_unit=mph" } else { "" };
    let data = fetch(&format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}\
         &current=temperature_2m,apparent_temperature,weather_code,is_day,wind_speed_10m\
         &daily=weather_code,temperature_2m_max,temperature_2m_min&timezone=auto&forecast_days=6{units}",
        place.latitude, place.longitude,
    ))
    .await?;
    parse(&data, &place.name, fahrenheit)
}

fn parse(data: &Value, place: &str, fahrenheit: bool) -> Option<Weather> {
    let now = &data["current"];
    let daily = &data["daily"];
    let column = |key: &str| daily[key].as_array().cloned().unwrap_or_default();
    let (dates, codes, highs, lows) =
        (column("time"), column("weather_code"), column("temperature_2m_max"), column("temperature_2m_min"));
    let days = (0..dates.len())
        .filter_map(|i| {
            Some(Day {
                date: dates[i].as_str()?.parse().ok()?,
                condition: Condition::from_code(codes.get(i)?.as_u64()?),
                high: highs.get(i)?.as_f64()?,
                low: lows.get(i)?.as_f64()?,
            })
        })
        .collect();
    Some(Weather {
        place: place.to_string(),
        temperature: now["temperature_2m"].as_f64()?,
        feels_like: now["apparent_temperature"].as_f64().unwrap_or_default(),
        wind: now["wind_speed_10m"].as_f64().unwrap_or_default(),
        condition: Condition::from_code(now["weather_code"].as_u64()?),
        day: now["is_day"].as_u64() != Some(0),
        days,
        fahrenheit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_forecast() {
        let data: Value = serde_json::from_str(
            r#"{"current":{"temperature_2m":14.9,"apparent_temperature":12.8,"weather_code":2,"is_day":0,"wind_speed_10m":5.0},
               "daily":{"time":["2026-10-30","2026-10-31"],"weather_code":[61,0],"temperature_2m_max":[16.1,18.0],"temperature_2m_min":[8.2,9.0]}}"#,
        )
        .unwrap();
        let w = parse(&data, "Budapest", false).unwrap();
        assert_eq!(w.condition, Condition::PartlyCloudy);
        assert!(!w.day);
        assert_eq!(w.days.len(), 2);
        assert_eq!(w.days[0].condition, Condition::Rain);
        assert_eq!(w.days[1].date, civil::date(2026, 10, 31));
        assert_eq!(Condition::from_code(95), Condition::Storm);
        assert_eq!(encoded("São Paulo"), "S%C3%A3o%20Paulo");
    }
}
