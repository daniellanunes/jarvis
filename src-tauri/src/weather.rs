//! Weather for the notch: Open-Meteo (free, no key). The city typed in
//! Ajustes → Notch is geocoded once, then the current conditions are cached
//! for 10 minutes.

use serde::Serialize;
use serde_json::Value;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::State;

use crate::Shared;

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WeatherNow {
    /// "São Paulo, SP"
    pub place: String,
    pub temp: f32,
    pub feels: f32,
    pub max: f32,
    pub min: f32,
    /// Chance of rain today, 0–100.
    pub rain_chance: u32,
    pub code: u32,
    pub is_day: bool,
    /// clear | partly | cloudy | fog | drizzle | rain | snow | storm
    pub kind: String,
    /// In Portuguese: "Céu limpo", "Chuva forte"…
    pub label: String,
}

struct Place {
    query: String,
    name: String,
    lat: f64,
    lon: f64,
}

static PLACE: LazyLock<Mutex<Option<Place>>> = LazyLock::new(|| Mutex::new(None));
static CACHE: LazyLock<Mutex<Option<(String, Instant, WeatherNow)>>> = LazyLock::new(|| Mutex::new(None));
const TTL: Duration = Duration::from_secs(600);

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())
}

/// WMO weather code → (kind, Portuguese label).
pub fn describe(code: u32) -> (&'static str, &'static str) {
    match code {
        0 => ("clear", "Céu limpo"),
        1 => ("partly", "Poucas nuvens"),
        2 => ("partly", "Parcialmente nublado"),
        3 => ("cloudy", "Nublado"),
        45 | 48 => ("fog", "Neblina"),
        51 | 53 | 55 | 56 | 57 => ("drizzle", "Garoa"),
        61 | 80 => ("rain", "Chuva fraca"),
        63 | 81 => ("rain", "Chuva"),
        65 | 82 | 66 | 67 => ("rain", "Chuva forte"),
        71 | 73 | 75 | 77 | 85 | 86 => ("snow", "Neve"),
        95 | 96 | 99 => ("storm", "Tempestade"),
        _ => ("cloudy", "Tempo instável"),
    }
}

async fn geocode(query: &str) -> Result<Place, String> {
    let mut url = reqwest::Url::parse("https://geocoding-api.open-meteo.com/v1/search").unwrap();
    url.query_pairs_mut()
        .append_pair("name", query)
        .append_pair("count", "1")
        .append_pair("language", "pt")
        .append_pair("format", "json");
    let v: Value = client()?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Clima: sem conexão ({e})"))?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let r = v["results"].get(0).ok_or_else(|| format!("Cidade não encontrada: {query}"))?;
    let name = r["name"].as_str().unwrap_or(query);
    let region = r["admin1"].as_str().unwrap_or("");
    Ok(Place {
        query: query.to_string(),
        name: if region.is_empty() || region == name { name.to_string() } else { format!("{name}, {region}") },
        lat: r["latitude"].as_f64().ok_or("Coordenadas inválidas")?,
        lon: r["longitude"].as_f64().ok_or("Coordenadas inválidas")?,
    })
}

pub async fn now(city: &str) -> Result<WeatherNow, String> {
    let city = city.trim();
    if city.is_empty() {
        return Err("Defina sua cidade em Ajustes → Notch.".into());
    }
    if let Some((c, at, w)) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        if c == city && at.elapsed() < TTL {
            return Ok(w.clone());
        }
    }
    let known = {
        let p = PLACE.lock().unwrap_or_else(|e| e.into_inner());
        p.as_ref().filter(|p| p.query.eq_ignore_ascii_case(city)).map(|p| (p.name.clone(), p.lat, p.lon))
    };
    let (name, lat, lon) = match known {
        Some(k) => k,
        None => {
            let p = geocode(city).await?;
            let out = (p.name.clone(), p.lat, p.lon);
            *PLACE.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
            out
        }
    };
    let mut url = reqwest::Url::parse("https://api.open-meteo.com/v1/forecast").unwrap();
    url.query_pairs_mut()
        .append_pair("latitude", &lat.to_string())
        .append_pair("longitude", &lon.to_string())
        .append_pair("current", "temperature_2m,apparent_temperature,weather_code,is_day")
        .append_pair("daily", "temperature_2m_max,temperature_2m_min,precipitation_probability_max")
        .append_pair("forecast_days", "1")
        .append_pair("timezone", "auto");
    let v: Value = client()?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Clima: sem conexão ({e})"))?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let cur = &v["current"];
    let code = cur["weather_code"].as_u64().unwrap_or(3) as u32;
    let (kind, label) = describe(code);
    let first = |k: &str| v["daily"][k].get(0).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    let w = WeatherNow {
        place: name,
        temp: cur["temperature_2m"].as_f64().unwrap_or(0.0) as f32,
        feels: cur["apparent_temperature"].as_f64().unwrap_or(0.0) as f32,
        max: first("temperature_2m_max"),
        min: first("temperature_2m_min"),
        rain_chance: first("precipitation_probability_max").round() as u32,
        code,
        is_day: cur["is_day"].as_u64().unwrap_or(1) == 1,
        kind: kind.into(),
        label: label.into(),
    };
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some((city.to_string(), Instant::now(), w.clone()));
    Ok(w)
}

/// Current weather for the city in the settings (or `city` when testing one).
#[tauri::command]
pub async fn weather_now(shared: State<'_, Shared>, city: Option<String>) -> Result<WeatherNow, String> {
    let city = city.unwrap_or_else(|| shared.settings.lock().unwrap().jarvis_weather_city.clone());
    now(&city).await
}

#[cfg(test)]
mod tests {
    use super::describe;

    #[test]
    fn weather_codes_read_as_words() {
        assert_eq!(describe(0), ("clear", "Céu limpo"));
        assert_eq!(describe(63).0, "rain");
        assert_eq!(describe(95).0, "storm");
        assert_eq!(describe(1234).0, "cloudy");
    }
}
