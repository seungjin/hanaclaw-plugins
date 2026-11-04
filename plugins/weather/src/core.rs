//! Pure logic for the `weather` ZeroClaw plugin.
//!
//! Parses the wttr.in JSON response and formats a concise current-weather
//! report. This module has no wit-bindgen dependency, so `cargo test` runs
//! on the host without a wasm toolchain.

use serde::Deserialize;

/// Arguments the model sends for the `weather` tool.
#[derive(Debug, Default, Deserialize)]
pub struct ToolInput {
    /// City name or location query accepted by wttr.in, e.g. "Seoul" or
    /// "New York".
    pub city: String,
}

/// Selected fields from wttr.in `/ format=j1`.
#[derive(Debug, Deserialize)]
struct CurrentCondition {
    #[serde(rename = "weatherDesc")]
    weather_desc: Vec<NameValue>,
    #[serde(rename = "temp_C")]
    temp_c: String,
    #[serde(rename = "FeelsLikeC")]
    feels_like_c: String,
    #[serde(rename = "windspeedKmph")]
    wind_speed_kmph: String,
    #[serde(rename = "winddir16Point")]
    wind_dir_16_point: String,
    humidity: String,
    #[serde(rename = "pressure")]
    pressure_mb: String,
    #[serde(rename = "visibility")]
    visibility_km: String,
    #[serde(rename = "uvIndex")]
    uv_index: String,
}

#[derive(Debug, Deserialize)]
struct NameValue {
    value: String,
}

#[derive(Debug, Deserialize)]
struct NearestArea {
    #[serde(rename = "areaName")]
    area_name: Vec<NameValue>,
    country: Vec<NameValue>,
}

#[derive(Debug, Deserialize)]
struct WttrResponse {
    #[serde(rename = "current_condition")]
    current_condition: Vec<CurrentCondition>,
    #[serde(rename = "nearest_area")]
    nearest_area: Vec<NearestArea>,
}

/// Format a weather report from raw wttr.in JSON.
///
/// The returned string clearly attributes the result to the weather plugin
/// and includes the source URL.
pub fn format_report(city: &str, json: &str) -> Result<String, String> {
    let parsed: WttrResponse =
        serde_json::from_str(json).map_err(|e| format!("invalid weather response: {e}"))?;

    let current = parsed
        .current_condition
        .first()
        .ok_or("weather response contains no current condition")?;
    let area = parsed
        .nearest_area
        .first()
        .ok_or("weather response contains no location")?;

    let area_name = area
        .area_name
        .first()
        .map(|nv| nv.value.as_str())
        .unwrap_or("Unknown location");
    let country = area
        .country
        .first()
        .map(|nv| nv.value.as_str())
        .unwrap_or("Unknown country");
    let description = current
        .weather_desc
        .first()
        .map(|nv| nv.value.as_str())
        .unwrap_or("unknown");

    let query = urlencode_city(city);
    let source_url = format!("https://wttr.in/{query}?format=j1");

    Ok(format!(
        "🌤️ Weather plugin result\n\n\
        Location: {area_name}, {country}\n\
        Condition: {description}\n\
        Temperature: {temp}°C (feels like {feels}°C)\n\
        Wind: {wind_dir} {wind_speed} km/h\n\
        Humidity: {humidity}%\n\
        Pressure: {pressure} mb\n\
        Visibility: {visibility} km\n\
        UV index: {uv}\n\n\
        Source: {source_url}",
        area_name = area_name,
        country = country,
        description = description,
        temp = current.temp_c,
        feels = current.feels_like_c,
        wind_dir = current.wind_dir_16_point,
        wind_speed = current.wind_speed_kmph,
        humidity = current.humidity,
        pressure = current.pressure_mb,
        visibility = current.visibility_km,
        uv = current.uv_index,
        source_url = source_url,
    ))
}

/// Percent-encode a city name for the wttr.in URL path.
///
/// wttr.in accepts spaces as underscores or encoded; underscores keep the
/// URL readable and are safe on the path.
fn urlencode_city(city: &str) -> String {
    city.trim().replace(' ', "_").replace(['/', '\\'], "")
}

/// Strip HTTP response headers and return the decoded body.
///
/// wttr.in answers `Transfer-Encoding: chunked` (over HTTP/1.1) with a
/// hex size line before the JSON; without decoding it the payload starts
/// with the chunk length, not `{`.
pub fn decode_http_response(response: &str) -> Result<String, String> {
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .ok_or("invalid HTTP response: no header/body delimiter")?;

    let chunked = headers.lines().skip(1).any(|line| {
        let line = line.to_ascii_lowercase();
        line.starts_with("transfer-encoding:") && line.contains("chunked")
    });

    if chunked {
        decode_chunked(body)
    } else {
        Ok(body.to_string())
    }
}

/// Decode an HTTP/1.1 chunked body into a plain string.
fn decode_chunked(body: &str) -> Result<String, String> {
    let mut decoded = String::new();
    let mut remaining = body;

    loop {
        // Each chunk: "<hex-size>[; ext]*\r\n<data>\r\n"
        let (size_line, rest) = remaining
            .split_once("\r\n")
            .ok_or("invalid chunked body: missing chunk size line")?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| format!("invalid chunked body: bad chunk size `{size_hex}`"))?;

        if size == 0 {
            break;
        }
        if rest.len() < size + 2 {
            return Err("invalid chunked body: truncated chunk data".to_string());
        }
        decoded.push_str(&rest[..size]);

        // Skip the chunk data plus its trailing \r\n.
        remaining = &rest[size + 2..];
    }

    Ok(decoded)
}

/// Run the tool against a wttr.in JSON response.
///
/// In the real wasm guest this function receives the JSON fetched by the
/// host over the WIT `sockets` interface; for host-run tests we pass the
/// body directly.
pub fn run(input: &ToolInput, json_response: &str) -> String {
    if input.city.trim().is_empty() {
        return "Error: city is required.".to_string();
    }
    match format_report(&input.city, json_response) {
        Ok(report) => report,
        Err(e) => format!("Error: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "current_condition": [
            {
                "FeelsLikeC": "7",
                "FeelsLikeF": "45",
                "cloudcover": "0",
                "humidity": "24",
                "observation_time": "07:52 AM",
                "precipInches": "0.0",
                "precipMM": "0.0",
                "pressure": "1021",
                "pressureInches": "30",
                "temp_C": "14",
                "temp_F": "58",
                "uvIndex": "1",
                "visibility": "10",
                "weatherCode": "113",
                "weatherDesc": [{"value": "Sunny"}],
                "weatherIconUrl": [{"value": ""}],
                "winddir16Point": "NW",
                "winddirDegree": "313",
                "windspeedKmph": "22",
                "windspeedMiles": "13"
            }
        ],
        "nearest_area": [
            {
                "areaName": [{"value": "Chongdong"}],
                "country": [{"value": "South Korea"}],
                "latitude": "37.57",
                "longitude": "126.98"
            }
        ],
        "request": [{"query": "Lat 37.57 and Lon 126.98", "type": "LatLon"}],
        "weather": []
    }"#;

    #[test]
    fn formats_a_wttr_report() {
        let input = ToolInput {
            city: "Seoul".to_string(),
        };
        let output = run(&input, SAMPLE);
        assert!(output.contains("Weather plugin result"));
        assert!(output.contains("Sunny"));
        assert!(output.contains("14°C"));
        assert!(output.contains("Source: https://wttr.in/Seoul?format=j1"));
    }

    #[test]
    fn rejects_empty_city() {
        let input = ToolInput {
            city: "   ".to_string(),
        };
        assert!(run(&input, SAMPLE).starts_with("Error: city is required"));
    }

    #[test]
    fn urlencodes_spaces_to_underscores() {
        assert_eq!(urlencode_city("New York"), "New_York");
    }

    #[test]
    fn strips_path_separator_characters() {
        assert_eq!(urlencode_city("a/b\\c"), "abc");
    }

    #[test]
    fn decodes_chunked_http_body() {
        let response = "HTTP/1.1 200 OK\r\n\
            Content-Type: text/plain\r\n\
            Transfer-Encoding: chunked\r\n\
            \r\n\
            4\r\n{\"a\"\r\n\
            3\r\n: 1\r\n\
            0\r\n\r\n";
        assert_eq!(decode_http_response(response).unwrap(), "{\"a\": 1");
    }

    #[test]
    fn returns_body_verbatim_without_chunking() {
        let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";
        assert_eq!(decode_http_response(response).unwrap(), "{}");
    }

    #[test]
    fn rejects_response_without_headers() {
        assert!(decode_http_response("99f9\r\n{").is_err());
    }
}
