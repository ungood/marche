//! OWS durations, as `wait`, `timeout` and retry delays write them: an ISO 8601
//! duration such as `PT1M30S`, or an object of `days`, `hours`, `minutes`,
//! `seconds` and `milliseconds`.

use std::time::Duration;

use serde_json::Value;

use crate::ows;

pub fn parse(value: &Value) -> Result<Duration, ows::Error> {
    match value {
        Value::String(text) => iso8601(text).ok_or_else(|| invalid(value)),
        Value::Object(fields) => {
            let mut millis = 0.0;
            for (unit, amount) in fields {
                let scale = match unit.as_str() {
                    "days" => 86_400_000.0,
                    "hours" => 3_600_000.0,
                    "minutes" => 60_000.0,
                    "seconds" => 1_000.0,
                    "milliseconds" => 1.0,
                    _ => return Err(invalid(value)),
                };
                let amount = amount
                    .as_f64()
                    .filter(|n| *n >= 0.0)
                    .ok_or_else(|| invalid(value))?;
                millis += amount * scale;
            }
            Ok(Duration::from_secs_f64(millis / 1_000.0))
        }
        _ => Err(invalid(value)),
    }
}

fn invalid(value: &Value) -> ows::Error {
    ows::Error::runtime("Invalid duration", format!("{value} isn't a duration."))
}

/// `P[nD][T[nH][nM][nS]]`, with an optional fraction on the last number. Weeks,
/// months and years aren't fixed lengths, so they aren't accepted.
fn iso8601(text: &str) -> Option<Duration> {
    let rest = text.strip_prefix('P')?;
    let (date, time) = match rest.split_once('T') {
        Some((date, time)) => (date, Some(time)),
        None => (rest, None),
    };
    let mut seconds = 0.0;
    let mut parts = 0;
    for (part, units) in [
        (date, &[('D', 86_400.0)][..]),
        (
            time.unwrap_or(""),
            &[('H', 3_600.0), ('M', 60.0), ('S', 1.0)][..],
        ),
    ] {
        let mut rest = part;
        for (unit, scale) in units {
            if let Some((number, after)) = rest.split_once(*unit) {
                let number: f64 = number.parse().ok().filter(|n: &f64| *n >= 0.0)?;
                seconds += number * scale;
                rest = after;
                parts += 1;
            }
        }
        if !rest.is_empty() {
            return None;
        }
    }
    (parts > 0 && time != Some("")).then(|| Duration::from_secs_f64(seconds))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_both_forms() {
        assert_eq!(parse(&json!("PT1M30S")), Ok(Duration::from_secs(90)));
        assert_eq!(parse(&json!("P1DT1H")), Ok(Duration::from_secs(90_000)));
        assert_eq!(parse(&json!("PT0.5S")), Ok(Duration::from_millis(500)));
        assert_eq!(
            parse(&json!({"minutes": 1, "milliseconds": 500})),
            Ok(Duration::from_millis(60_500))
        );
    }

    #[test]
    fn rejects_what_isnt_a_duration() {
        for value in [
            json!("1m"),
            json!("P"),
            json!("PT"),
            json!("P1W"),
            json!({"weeks": 1}),
            json!(5),
        ] {
            assert!(parse(&value).is_err(), "{value}");
        }
    }
}
