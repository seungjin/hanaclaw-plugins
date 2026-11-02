//! Host-run integration test over the pure core: no wasm toolchain needed.

use weather::core::{ToolInput, run};

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
fn reports_current_weather_for_seoul() {
    let input = ToolInput {
        city: "Seoul".to_string(),
    };
    let output = run(&input, SAMPLE);
    assert!(
        output.contains("Weather plugin result"),
        "missing attribution"
    );
    assert!(
        output.contains("Chongdong, South Korea"),
        "missing resolved location"
    );
    assert!(output.contains("Sunny"), "missing condition");
    assert!(
        output.contains("Source: https://wttr.in/Seoul?format=j1"),
        "missing source URL"
    );
}

#[test]
fn urlencodes_multiword_city_names() {
    let input = ToolInput {
        city: "New York".to_string(),
    };
    let output = run(&input, SAMPLE);
    assert!(output.contains("Source: https://wttr.in/New_York?format=j1"));
}
