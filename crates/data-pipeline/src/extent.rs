//! Geographic extent for the fixed-camera bakes: the original single-borough
//! **Manhattan** build, or the full five-borough **NYC** city.
//!
//! Several camera bakers (`amnesty`, `dot`, `cameras_dahir`) guard their input
//! with a loose lat/lon bounding box and — where the source carries one — a
//! borough/area name. Both were hardcoded to Manhattan. `Extent` parameterizes
//! that guard so the same baker emits a Manhattan-only or a citywide layer; the
//! ENU projection origin (`GeoOrigin::MANHATTAN`) is unchanged and remains the
//! canonical reference point for the whole city.
//!
//! Default is `Manhattan` (the deployed build); pass `nyc` (or `citywide`/`all`)
//! to opt a bake into the five-borough extent.

/// Which slice of the city a camera bake should keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extent {
    /// Manhattan island only (the original build).
    Manhattan,
    /// All five boroughs.
    Nyc,
}

impl Extent {
    /// Parse the optional CLI extent argument; anything unrecognized (or absent)
    /// falls back to `Manhattan` so existing invocations are unchanged.
    pub fn parse(arg: Option<&str>) -> Self {
        match arg.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("nyc") | Some("citywide") | Some("all") | Some("5") | Some("five") => Extent::Nyc,
            _ => Extent::Manhattan,
        }
    }

    /// Loose lat/lon bounding box `(lat_min, lat_max, lon_min, lon_max)` — a cheap
    /// prefilter. For Manhattan it brackets the island + immediate waterfront; for
    /// NYC it brackets all five boroughs (Staten Island's south tip to the north
    /// Bronx, the SI west shore to eastern Queens).
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        match self {
            Extent::Manhattan => (40.698, 40.882, -74.022, -73.906),
            Extent::Nyc => (40.477, 40.925, -74.270, -73.680),
        }
    }

    /// Is `(lat, lon)` inside the extent's bounding box?
    pub fn contains_latlon(&self, lat: f64, lon: f64) -> bool {
        let (la0, la1, lo0, lo1) = self.bbox();
        (la0..=la1).contains(&lat) && (lo0..=lo1).contains(&lon)
    }

    /// Does this extent accept a row tagged with the given borough name? Manhattan
    /// keeps only `Manhattan`; NYC keeps all five (the canonical NYC Open Data
    /// `BoroName`/`area` spellings).
    pub fn accepts_borough(&self, boro: &str) -> bool {
        match self {
            Extent::Manhattan => boro == "Manhattan",
            Extent::Nyc => matches!(
                boro,
                "Manhattan" | "Brooklyn" | "Queens" | "Bronx" | "Staten Island"
            ),
        }
    }

    /// Human-readable label for log lines.
    pub fn label(&self) -> &'static str {
        match self {
            Extent::Manhattan => "Manhattan",
            Extent::Nyc => "NYC (5 boroughs)",
        }
    }

    /// ACE route-base key for matching the official ACE route list against GTFS
    /// `route_short_name`s. The base is the leading **alphabetic prefix
    /// (uppercased)** plus the following run of digits:
    /// `"M15+" -> "M15"`, `"BX41+" -> "BX41"`, `"S79+" -> "S79"`,
    /// `"M14A-SBS" -> "M14"`. Uppercasing is required because GTFS spells the
    /// Bronx `"Bx41"` while the ACE list uses `"BX41"`.
    ///
    /// For the `Manhattan` extent only `M…` routes match (parity with the
    /// single-borough build); for `Nyc`, all five boroughs' routes match.
    /// Returns `None` for an unparseable name (no leading letter or no digits).
    pub fn route_base(&self, name: &str) -> Option<String> {
        let prefix: String = name.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        if prefix.is_empty() {
            return None;
        }
        let prefix = prefix.to_ascii_uppercase();
        if *self == Extent::Manhattan && prefix != "M" {
            return None;
        }
        let digits: String = name[prefix.len()..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        (!digits.is_empty()).then(|| format!("{prefix}{digits}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_defaults_to_manhattan() {
        assert_eq!(Extent::parse(None), Extent::Manhattan);
        assert_eq!(Extent::parse(Some("manhattan")), Extent::Manhattan);
        assert_eq!(Extent::parse(Some("garbage")), Extent::Manhattan);
        assert_eq!(Extent::parse(Some("nyc")), Extent::Nyc);
        assert_eq!(Extent::parse(Some("NYC")), Extent::Nyc);
        assert_eq!(Extent::parse(Some("citywide")), Extent::Nyc);
    }

    #[test]
    fn manhattan_bbox_excludes_other_boroughs() {
        // A point in downtown Brooklyn (~40.69, -73.99) is inside the NYC box but
        // outside the Manhattan box.
        assert!(!Extent::Manhattan.contains_latlon(40.690, -73.990));
        assert!(Extent::Nyc.contains_latlon(40.690, -73.990));
        // Midtown Manhattan is inside both.
        assert!(Extent::Manhattan.contains_latlon(40.754, -73.984));
        assert!(Extent::Nyc.contains_latlon(40.754, -73.984));
    }

    #[test]
    fn borough_filter_matches_extent() {
        assert!(Extent::Manhattan.accepts_borough("Manhattan"));
        assert!(!Extent::Manhattan.accepts_borough("Brooklyn"));
        for b in ["Manhattan", "Brooklyn", "Queens", "Bronx", "Staten Island"] {
            assert!(Extent::Nyc.accepts_borough(b), "NYC should accept {b}");
        }
        assert!(!Extent::Nyc.accepts_borough("Jersey City"));
    }

    #[test]
    fn route_base_normalizes_prefix_and_gates_by_extent() {
        // NYC accepts all boroughs; GTFS "Bx41" and ACE "BX41" both normalize to "BX41".
        assert_eq!(Extent::Nyc.route_base("BX41+"), Some("BX41".into()));
        assert_eq!(Extent::Nyc.route_base("Bx41"), Some("BX41".into()));
        assert_eq!(Extent::Nyc.route_base("B44+"), Some("B44".into()));
        assert_eq!(Extent::Nyc.route_base("Q44+"), Some("Q44".into()));
        assert_eq!(Extent::Nyc.route_base("S79+"), Some("S79".into()));
        assert_eq!(Extent::Nyc.route_base("M14A-SBS"), Some("M14".into()));
        // Manhattan extent keeps only M-routes (parity with the old manhattan_base).
        assert_eq!(Extent::Manhattan.route_base("M15+"), Some("M15".into()));
        assert_eq!(Extent::Manhattan.route_base("BX41+"), None);
        assert_eq!(Extent::Manhattan.route_base("B44+"), None);
        // Unparseable.
        assert_eq!(Extent::Nyc.route_base("+++"), None);
        assert_eq!(Extent::Nyc.route_base("SIM"), None); // no digits
    }

    #[test]
    fn nyc_bbox_brackets_all_boroughs() {
        // Representative interior points per borough.
        let pts = [
            (40.754, -73.984), // Manhattan, Midtown
            (40.650, -73.950), // Brooklyn
            (40.728, -73.794), // Queens, Flushing-ish
            (40.844, -73.865), // Bronx
            (40.580, -74.150), // Staten Island
        ];
        for (lat, lon) in pts {
            assert!(Extent::Nyc.contains_latlon(lat, lon), "NYC bbox should hold {lat},{lon}");
        }
    }
}
