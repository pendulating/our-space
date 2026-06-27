//! NYC address geocoding via the key-free **NYC GeoSearch** API (a Pelias instance
//! on DCP's PAD address data — the same authoritative source as Geosupport). Lets a
//! visitor set the A→B endpoints and the walkshed center by *typing an address*, in
//! addition to clicking the map.
//!
//! Cross-platform via `ehttp` (browser `fetch` on wasm, background threads on
//! native), so the same code path runs in the public web build and native dev. A
//! request's results land back in the ECS through a shared inbox that `geocode_tick`
//! drains each frame — no async runtime in the Bevy loop, no JS bridge.
//!
//! Privacy note: the *typed query text* and — when you click the map to drop a pin —
//! that single clicked coordinate are sent to DCP's geocoder (the click is reverse-
//! geocoded to label the nearest address). The computed route itself still never leaves.

use std::sync::{Arc, Mutex};

use bevy::prelude::*;

/// Autocomplete endpoint (no key, CORS-enabled).
const GEOSEARCH_URL: &str = "https://geosearch.planninglabs.nyc/v2/autocomplete";
/// Reverse endpoint — nearest address to a clicked map point.
const GEOSEARCH_REVERSE_URL: &str = "https://geosearch.planninglabs.nyc/v2/reverse";
/// Bias suggestions toward Manhattan (matches `GeoOrigin::MANHATTAN`).
const FOCUS_LAT: f64 = 40.7831;
const FOCUS_LON: f64 = -73.9712;
/// Manhattan bbox — drop out-of-borough candidates (the default Manhattan build).
const MAN_LAT: (f64, f64) = (40.697, 40.882);
const MAN_LON: (f64, f64) = (-74.022, -73.905);
/// All-five-borough bbox (matches the citywide street graph extent) — the geocoder
/// uses this instead of the Manhattan box when the citywide build is active, so a
/// Brooklyn/Queens click reverse-geocodes to its real address instead of being
/// dropped and falling back to a stray Manhattan candidate.
const NYC_LAT: (f64, f64) = (40.49, 40.92);
const NYC_LON: (f64, f64) = (-74.27, -73.69);

/// Set once at startup from `CityScope`: widens the geocoder's accept-box to all five
/// boroughs. An atomic so the free worker-dispatch functions can read it without
/// threading the flag through every call.
static CITYWIDE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Switch the geocoder between the Manhattan and the five-borough accept-box.
pub fn set_citywide(on: bool) {
    CITYWIDE.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Is a WGS84 point inside the accept-box for the given scope (pure — testable
/// without the global)? `citywide` picks the five-borough box over the Manhattan one.
fn in_box(lat: f64, lon: f64, citywide: bool) -> bool {
    let (lat_b, lon_b) = if citywide { (NYC_LAT, NYC_LON) } else { (MAN_LAT, MAN_LON) };
    lat >= lat_b.0 && lat <= lat_b.1 && lon >= lon_b.0 && lon <= lon_b.1
}

/// Is a WGS84 point inside the *active* build's accept-box (reads the global flag)?
fn in_scope(lat: f64, lon: f64) -> bool {
    in_box(lat, lon, CITYWIDE.load(std::sync::atomic::Ordering::Relaxed))
}
/// Type-ahead debounce + minimum characters before a query fires.
const DEBOUNCE_SECS: f32 = 0.28;
const MIN_CHARS: usize = 3;
const MAX_RESULTS: usize = 6;

/// One geocoded candidate: a display label + its WGS84 position.
#[derive(Clone, Debug)]
pub struct GeoResult {
    pub label: String,
    pub lat: f64,
    pub lon: f64,
}

/// Which input a field drives.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Walkshed,
    Start,
    Dest,
}

/// Editable, debounced autocomplete state for one address box.
#[derive(Default)]
pub struct GeoField {
    /// Text currently in the box (bound to the egui `TextEdit`).
    pub query: String,
    /// Autocomplete candidates for the current query.
    pub results: Vec<GeoResult>,
    /// The candidate the user committed (cleared when the text is edited again).
    pub resolved: Option<GeoResult>,
    /// A request is in flight (drives a "…" hint).
    pub loading: bool,
    /// The last completed query came back with zero Manhattan matches.
    pub no_match: bool,
    sent_query: String, // last query dispatched (dedup)
    debounce: f32,      // seconds until the pending query fires (<= 0 = idle)
    req_id: u64,        // newest request id (stale responses ignored)
}

impl GeoField {
    /// Call when the text changed: arm the debounce and drop any stale resolution.
    pub fn on_edit(&mut self) {
        self.debounce = DEBOUNCE_SECS;
        self.resolved = None;
        self.no_match = false;
        if self.query.trim().len() < MIN_CHARS {
            self.results.clear();
            self.loading = false;
        }
    }

    /// Reset the box completely (the ✕ clear button).
    pub fn clear(&mut self) {
        *self = GeoField::default();
    }

    /// Apply a reverse-geocode result (from a map click): resolve directly to the
    /// nearest address with no dropdown. Empty → flag "no match" (e.g. a click off the
    /// addressable grid). Display-only — the point is already placed by the click.
    fn resolve_reverse(&mut self, results: Vec<GeoResult>) {
        self.loading = false;
        self.results.clear();
        self.debounce = 0.0;
        match results.into_iter().next() {
            Some(top) => {
                self.query = top.label.clone();
                self.sent_query = top.label.clone(); // don't re-search the filled text
                self.resolved = Some(top);
                self.no_match = false;
            }
            None => self.no_match = true,
        }
    }
}

struct Incoming {
    field: Field,
    req_id: u64,
    results: Vec<GeoResult>,
    /// A reverse (map-click) lookup — resolve straight to the nearest address.
    reverse: bool,
}

/// The geocoder resource: the three address fields + a pick the panel hands off to
/// `apply_geocode`, plus the cross-thread inbox results arrive on.
#[derive(Resource)]
pub struct Geocoder {
    pub walkshed: GeoField,
    pub start: GeoField,
    pub dest: GeoField,
    /// Set by the panel when a result row is clicked; consumed by `apply_geocode`.
    pub picked: Option<(Field, GeoResult)>,
    /// Set by the panel's A↔B swap button; consumed by `apply_geocode`.
    pub swap: bool,
    /// Set by the "Surprise me" buttons; consumed by `apply_geocode`, which drops a
    /// random walkshed center / A→B pair on the street graph.
    pub random_walkshed: bool,
    pub random_route: bool,
    /// Set by a field's ✕ clear button; consumed by `apply_geocode` (which also drops
    /// the corresponding endpoint / walkshed).
    pub cleared: Option<Field>,
    inbox: Arc<Mutex<Vec<Incoming>>>,
    next_id: u64,
}

impl Default for Geocoder {
    fn default() -> Self {
        Geocoder {
            walkshed: GeoField::default(),
            start: GeoField::default(),
            dest: GeoField::default(),
            picked: None,
            swap: false,
            random_walkshed: false,
            random_route: false,
            cleared: None,
            inbox: Arc::new(Mutex::new(Vec::new())),
            next_id: 1,
        }
    }
}

impl Geocoder {
    pub fn field_mut(&mut self, f: Field) -> &mut GeoField {
        match f {
            Field::Walkshed => &mut self.walkshed,
            Field::Start => &mut self.start,
            Field::Dest => &mut self.dest,
        }
    }

    /// Reverse-geocode a clicked map point and fill `field` with the nearest address
    /// (bidirectional input: clicking the map updates the address box, mirroring how
    /// typing an address drops a pin). The point is already placed by the caller — this
    /// only labels the box, so it does **not** set `picked`.
    pub fn reverse_lookup(&mut self, field: Field, lat: f64, lon: f64) {
        let id = self.next_id;
        self.next_id += 1;
        let inbox = self.inbox.clone();
        {
            let f = self.field_mut(field);
            f.req_id = id; // newer than any in-flight forward search → that one goes stale
            f.loading = true;
            f.debounce = 0.0; // cancel a queued autocomplete for this field
            f.no_match = false;
        }
        reverse_dispatch(field, lat, lon, id, inbox);
    }
}

/// Per-frame: drain finished requests into their field, then tick each field's
/// debounce and dispatch a fresh autocomplete request when the typing settles.
pub fn geocode_tick(time: Res<Time>, mut geo: ResMut<Geocoder>) {
    // 1) Drain finished requests into their fields (ignoring stale responses).
    let incoming: Vec<Incoming> = std::mem::take(&mut *geo.inbox.lock().unwrap());
    for inc in incoming {
        let f = geo.field_mut(inc.field);
        if inc.req_id == f.req_id {
            if inc.reverse {
                f.resolve_reverse(inc.results);
            } else {
                f.no_match = inc.results.is_empty() && f.query.trim().len() >= MIN_CHARS;
                f.results = inc.results;
                f.loading = false;
            }
        }
    }
    // 2) Tick debounce + dispatch.
    let dt = time.delta_secs();
    let inbox = geo.inbox.clone();
    for fid in [Field::Walkshed, Field::Start, Field::Dest] {
        let dispatch_q: Option<(String, u64)> = {
            let next_id = geo.next_id;
            let f = geo.field_mut(fid);
            if f.debounce <= 0.0 {
                None
            } else {
                f.debounce -= dt;
                if f.debounce > 0.0 {
                    None
                } else {
                    let q = f.query.trim().to_string();
                    if q.len() < MIN_CHARS || q == f.sent_query {
                        None
                    } else {
                        f.sent_query = q.clone();
                        f.req_id = next_id;
                        f.loading = true;
                        Some((q, next_id))
                    }
                }
            }
        };
        if let Some((q, id)) = dispatch_q {
            dispatch(fid, &q, id, inbox.clone());
            geo.next_id += 1;
        }
    }
}

/// Fire one autocomplete request; the callback (browser microtask / native thread)
/// parses + pushes into the shared inbox.
fn dispatch(field: Field, query: &str, req_id: u64, inbox: Arc<Mutex<Vec<Incoming>>>) {
    let url = format!(
        "{GEOSEARCH_URL}?text={}&focus.point.lat={FOCUS_LAT}&focus.point.lon={FOCUS_LON}",
        encode(query)
    );
    ehttp::fetch(ehttp::Request::get(url), move |result| {
        let results = match result {
            Ok(resp) if resp.ok => parse_geosearch(&resp.bytes),
            _ => Vec::new(),
        };
        if let Ok(mut inbox) = inbox.lock() {
            inbox.push(Incoming { field, req_id, results, reverse: false });
        }
    });
}

/// Fire one reverse request (clicked point → nearest address); push the top result.
fn reverse_dispatch(
    field: Field,
    lat: f64,
    lon: f64,
    req_id: u64,
    inbox: Arc<Mutex<Vec<Incoming>>>,
) {
    let url = format!("{GEOSEARCH_REVERSE_URL}?point.lat={lat}&point.lon={lon}&size=1");
    ehttp::fetch(ehttp::Request::get(url), move |result| {
        let results = match result {
            Ok(resp) if resp.ok => parse_geosearch(&resp.bytes),
            _ => Vec::new(),
        };
        if let Ok(mut inbox) = inbox.lock() {
            inbox.push(Incoming { field, req_id, results, reverse: true });
        }
    });
}

/// Parse a GeoSearch GeoJSON FeatureCollection → candidates inside the active
/// build's accept-box (Manhattan, or all five boroughs when citywide).
fn parse_geosearch(bytes: &[u8]) -> Vec<GeoResult> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let Some(feats) = v.get("features").and_then(|f| f.as_array()) else {
        return out;
    };
    for f in feats {
        let coords = f
            .get("geometry")
            .and_then(|g| g.get("coordinates"))
            .and_then(|c| c.as_array());
        let label = f
            .get("properties")
            .and_then(|p| p.get("label"))
            .and_then(|l| l.as_str());
        let (Some(c), Some(label)) = (coords, label) else { continue };
        if c.len() < 2 {
            continue;
        }
        let (Some(lon), Some(lat)) = (c[0].as_f64(), c[1].as_f64()) else { continue };
        if !in_scope(lat, lon) {
            continue; // outside the active build's accept-box (Manhattan, or all of NYC)
        }
        out.push(GeoResult { label: label.to_string(), lat, lon });
        if out.len() >= MAX_RESULTS {
            break;
        }
    }
    out
}

/// Minimal percent-encoding for the query string (stdlib only).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_features_and_filters_to_manhattan() {
        let json = br#"{"features":[
            {"geometry":{"coordinates":[-73.9857,40.7484]},"properties":{"label":"350 5 Ave, Manhattan"}},
            {"geometry":{"coordinates":[-73.9442,40.6782]},"properties":{"label":"Brooklyn pl"}}
        ]}"#;
        let r = parse_geosearch(json);
        assert_eq!(r.len(), 1, "the Brooklyn candidate is filtered out");
        assert_eq!(r[0].label, "350 5 Ave, Manhattan");
        assert!((r[0].lat - 40.7484).abs() < 1e-6 && (r[0].lon + 73.9857).abs() < 1e-6);
    }

    #[test]
    fn encode_escapes_spaces_and_punct() {
        assert_eq!(encode("350 5th Ave"), "350%205th%20Ave");
    }

    #[test]
    fn accept_box_widens_to_all_boroughs_when_citywide() {
        // A south-Brooklyn point (Coney Island ~40.575,-73.979) is rejected by the
        // Manhattan box but accepted by the five-borough box — the reverse-geocode fix.
        assert!(!in_box(40.575, -73.979, false), "Brooklyn rejected in the Manhattan build");
        assert!(in_box(40.575, -73.979, true), "Brooklyn accepted in the citywide build");
        // A Queens point (Flushing ~40.759,-73.830) likewise.
        assert!(!in_box(40.759, -73.830, false));
        assert!(in_box(40.759, -73.830, true));
        // Manhattan stays in-box under both scopes; a point west of both boxes (deep
        // NJ, lon −74.30) stays out under both. (The reverse API is NYC-only regardless,
        // so the coarse box is just a sanity clamp.)
        assert!(in_box(40.7484, -73.9857, false) && in_box(40.7484, -73.9857, true));
        assert!(!in_box(40.72, -74.30, false) && !in_box(40.72, -74.30, true));
    }

    #[test]
    fn reverse_result_resolves_field_without_dropdown() {
        // A map click resolves straight to the nearest address (no candidate list),
        // and arms nothing that would re-search the filled text.
        let mut f = GeoField::default();
        f.loading = true;
        f.resolve_reverse(vec![GeoResult {
            label: "350 5 Ave, Manhattan".into(),
            lat: 40.7484,
            lon: -73.9857,
        }]);
        assert_eq!(f.query, "350 5 Ave, Manhattan");
        assert!(f.resolved.is_some(), "click resolves the box directly");
        assert!(f.results.is_empty(), "no autocomplete dropdown for a click");
        assert_eq!(f.sent_query, f.query, "won't re-search the filled address");
        assert!(!f.loading && !f.no_match);
    }

    #[test]
    fn reverse_with_no_match_flags_the_box() {
        let mut f = GeoField::default();
        f.resolve_reverse(vec![]);
        assert!(f.no_match, "a click off the addressable grid flags no-match");
        assert!(f.resolved.is_none());
    }
}
