//! Opening apps on the phone: the `app_launch` tool engine.
//!
//! One rule shapes everything here: a destination is data, never code.
//! The launch is an argv array (`gio open <uri>`) with no shell anywhere
//! near it, and the URI is built by percent-encoding the destination
//! into a `geo:` query, so shell metacharacters, a second URI, or a
//! scheme smuggled into the place name all stay inert text inside the
//! query.
//!
//! Tonight the allowlist has exactly one entry: `maps` (GNOME Maps, the
//! phone's registered `geo:` handler, verified in KEY-TEST-1-PHONE-RUN).
//! Any other app is refused before anything runs. Widening the list is
//! an operator decision, not a model decision.

use std::process::Command;

/// The apps `app_launch` may open. One entry tonight, by design.
pub const ALLOWED_APPS: [&str; 1] = ["maps"];

/// A launch, fully decided: the exact argv to run. Kept separate from
/// running it so the decision is unit-testable without a phone (or gio).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub app: String,
    pub uri: String,
    pub program: String,
    pub args: Vec<String>,
}

/// Decide a launch, or refuse with the reason the model should see.
pub fn build_launch(app: &str, destination: &str) -> Result<Launch, String> {
    let app = app.trim();
    if !ALLOWED_APPS.contains(&app) {
        return Err(format!(
            "app `{app}` is not on the launch allowlist (allowed: {})",
            ALLOWED_APPS.join(", ")
        ));
    }
    let uri = geo_uri(destination)?;
    Ok(Launch {
        app: app.to_string(),
        uri: uri.clone(),
        program: "gio".to_string(),
        args: vec!["open".to_string(), uri],
    })
}

/// The `geo:` URI for a destination: a point when the input parses as
/// `lat,lon`, otherwise a place search in the form verified against
/// GNOME Maps on the phone (`geo:0,0?q=San%20Francisco`).
pub fn geo_uri(destination: &str) -> Result<String, String> {
    let dest = destination.trim();
    if dest.is_empty() {
        return Err("destination must be a non-empty place name or `lat,lon` coordinates".to_string());
    }
    if dest.len() > 256 {
        return Err("destination is too long (256 characters at most)".to_string());
    }
    if dest.chars().any(|c| c.is_control()) {
        return Err("destination must not contain control characters".to_string());
    }
    if let Some((lat, lon)) = parse_coordinates(dest) {
        return Ok(format!("geo:{lat},{lon}"));
    }
    Ok(format!("geo:0,0?q={}", percent_encode(dest)))
}

/// Parse `lat,lon` (spaces optional). Both halves must be real numbers
/// in range; anything else is a place name, not coordinates — a comma
/// inside a name ("Paris, France") fails the float parse, and a second
/// comma is never coordinates either.
fn parse_coordinates(text: &str) -> Option<(f64, f64)> {
    let (a, b) = text.split_once(',')?;
    if b.contains(',') {
        return None;
    }
    let lat: f64 = a.trim().parse().ok()?;
    let lon: f64 = b.trim().parse().ok()?;
    if !lat.is_finite() || !lon.is_finite() {
        return None;
    }
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    Some((lat, lon))
}

/// Percent-encode per RFC 3986: unreserved bytes pass through, every
/// other byte becomes %XX — including `/`, `?`, `&`, `;` and any scheme
/// text, so the destination can never grow a second URI in the query.
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for &byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => {
                out.push('%');
                out.push(char::from_digit((byte >> 4) as u32, 16).unwrap().to_ascii_uppercase());
                out.push(char::from_digit((byte & 0xF) as u32, 16).unwrap().to_ascii_uppercase());
            }
        }
    }
    out
}

/// Run a decided launch. argv only — no shell — so the URI reaches
/// `gio` as exactly one argument, exactly as built.
pub fn run_launch(launch: &Launch) -> Result<std::process::Output, String> {
    Command::new(&launch.program)
        .args(&launch.args)
        .output()
        .map_err(|e| format!("could not run `{}`: {e}", launch.program))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_names_become_geo_queries() {
        assert_eq!(geo_uri("San Francisco").unwrap(), "geo:0,0?q=San%20Francisco");
        assert_eq!(geo_uri("  San Francisco  ").unwrap(), "geo:0,0?q=San%20Francisco");
    }

    #[test]
    fn coordinates_become_geo_points() {
        assert_eq!(geo_uri("37.422,-122.084").unwrap(), "geo:37.422,-122.084");
        assert_eq!(geo_uri("37.422, -122.084").unwrap(), "geo:37.422,-122.084");
        // Out-of-range numbers are a place name, not a point.
        assert!(geo_uri("123,456").unwrap().starts_with("geo:0,0?q="));
        // A comma inside a name is not coordinates.
        assert_eq!(geo_uri("Paris, France").unwrap(), "geo:0,0?q=Paris%2C%20France");
    }

    #[test]
    fn only_maps_may_launch() {
        assert!(build_launch("maps", "San Francisco").is_ok());
        let err = build_launch("browser", "San Francisco").unwrap_err();
        assert!(err.contains("not on the launch allowlist"), "got: {err}");
        let err = build_launch("", "San Francisco").unwrap_err();
        assert!(err.contains("not on the launch allowlist"), "got: {err}");
    }

    #[test]
    fn a_destination_cannot_escape_the_geo_intent() {
        // Shell metacharacters are encoded into inert query text, and
        // the launch is an argv array, never a shell string.
        let launch = build_launch("maps", "x; rm -rf / $(id)").unwrap();
        assert_eq!(launch.program, "gio");
        assert_eq!(launch.args.len(), 2);
        assert_eq!(launch.args[0], "open");
        assert!(launch.args[1].starts_with("geo:0,0?q="));
        assert!(!launch.args[1].contains(';') && !launch.args[1].contains('$') && !launch.args[1].contains(' '));
        // A second URI smuggled in as the destination stays a query.
        let launch = build_launch("maps", "geo:1,2").unwrap();
        assert_eq!(launch.args[1], "geo:0,0?q=geo%3A1%2C2");
        let launch = build_launch("maps", "https://evil.example/x").unwrap();
        assert!(launch.args[1].starts_with("geo:0,0?q=https%3A%2F%2F"));
        // Empty and control-character destinations are refused outright.
        assert!(build_launch("maps", "   ").is_err());
        assert!(build_launch("maps", "a\nb").is_err());
    }
}
