//! Cross-platform asset loading.
//!
//! The baked `.postcard` layers are loaded through Bevy's `AssetServer` so the
//! same code path works on native (reads `assets/`) and on the web (fetches over
//! HTTP). Each layer is a transparent newtype `Asset` wrapper around the sim-core
//! type; postcard newtype encoding is transparent, so the generic loader just
//! `postcard::from_bytes`-es the wrapper.

use std::marker::PhantomData;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;
use serde::de::DeserializeOwned;

use sim_core::assets::{
    AceCorridorLayer, AlprReaderLayer, BoroughOutline, BuildingFootprints, BusDayLayer,
    CctvCameraLayer, DashcamFieldLayer, EquityLayer, FixedSensorLayer, GraphAsset, HeatmapLayer,
    LandmarkMassing, LinkNycLayer, NeighborhoodLayer, RobotabilityField, TaxiDayLayer, TeslaField,
    VehicleRoutesLayer,
};

macro_rules! postcard_asset {
    ($name:ident, $inner:ty) => {
        #[derive(Asset, TypePath, serde::Deserialize)]
        pub struct $name(pub $inner);
    };
}

postcard_asset!(GraphAssetRes, GraphAsset);
postcard_asset!(CamerasRes, FixedSensorLayer);
postcard_asset!(CctvRes, CctvCameraLayer);
postcard_asset!(AceRes, AceCorridorLayer);
postcard_asset!(HeatmapRes, HeatmapLayer);
postcard_asset!(EquityRes, EquityLayer);
postcard_asset!(DashcamFieldRes, DashcamFieldLayer);
postcard_asset!(AlprRes, AlprReaderLayer);
postcard_asset!(DotRes, FixedSensorLayer);
postcard_asset!(VehicleRoutesRes, VehicleRoutesLayer);
postcard_asset!(NeighborhoodRes, NeighborhoodLayer);
postcard_asset!(BusDayRes, BusDayLayer);
postcard_asset!(TaxiDayRes, TaxiDayLayer);
postcard_asset!(RobotabilityRes, RobotabilityField);
postcard_asset!(TeslaFieldRes, TeslaField);
postcard_asset!(BoroughRes, BoroughOutline);
postcard_asset!(FootprintsRes, BuildingFootprints);
// Parks reuse the flat-polygon footprint payload, under their own extension so the
// loader resolves them distinctly (the app tints them green, not building-gray).
postcard_asset!(ParksRes, BuildingFootprints);
// Pedestrian plazas — same flat-polygon payload, rendered as a concrete fill + hatch.
postcard_asset!(PlazaRes, BuildingFootprints);
postcard_asset!(LandmarkRes, LandmarkMassing);
postcard_asset!(LinkNycRes, LinkNycLayer);

/// Generic loader for any postcard-encoded `Asset` newtype. Each instance owns a
/// distinct file extension so the right loader resolves per asset type (a single
/// shared extension is ambiguous across types and silently mis-decodes).
#[derive(TypePath)]
pub struct PostcardLoader<A: TypePath> {
    ext: &'static str,
    _marker: PhantomData<fn() -> A>,
}

impl<A: TypePath> PostcardLoader<A> {
    pub fn new(ext: &'static str) -> Self {
        PostcardLoader { ext, _marker: PhantomData }
    }
}

impl<A: Asset + DeserializeOwned> AssetLoader for PostcardLoader<A> {
    type Asset = A;
    type Settings = ();
    type Error = anyhow::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        _ctx: &mut LoadContext<'_>,
    ) -> Result<A, anyhow::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(postcard::from_bytes(&bytes)?)
    }

    fn extensions(&self) -> &[&str] {
        std::slice::from_ref(&self.ext)
    }
}

/// Register all asset types + loaders (distinct extensions per type).
pub fn register(app: &mut App) {
    app.init_asset::<GraphAssetRes>()
        .init_asset::<CamerasRes>()
        .init_asset::<CctvRes>()
        .init_asset::<AceRes>()
        .init_asset::<HeatmapRes>()
        .init_asset::<EquityRes>()
        .init_asset::<DashcamFieldRes>()
        .init_asset::<AlprRes>()
        .init_asset::<DotRes>()
        .init_asset::<VehicleRoutesRes>()
        .init_asset::<NeighborhoodRes>()
        .init_asset::<BusDayRes>()
        .init_asset::<TaxiDayRes>()
        .init_asset::<RobotabilityRes>()
        .init_asset::<TeslaFieldRes>()
        .init_asset::<BoroughRes>()
        .init_asset::<FootprintsRes>()
        .init_asset::<ParksRes>()
        .init_asset::<PlazaRes>()
        .init_asset::<LandmarkRes>()
        .init_asset::<LinkNycRes>()
        .register_asset_loader(PostcardLoader::<GraphAssetRes>::new("osgraph"))
        .register_asset_loader(PostcardLoader::<CamerasRes>::new("oscam"))
        .register_asset_loader(PostcardLoader::<CctvRes>::new("oscctv"))
        .register_asset_loader(PostcardLoader::<AceRes>::new("osace"))
        .register_asset_loader(PostcardLoader::<HeatmapRes>::new("osheat"))
        .register_asset_loader(PostcardLoader::<EquityRes>::new("osequity"))
        .register_asset_loader(PostcardLoader::<DashcamFieldRes>::new("osfield"))
        .register_asset_loader(PostcardLoader::<AlprRes>::new("osalpr"))
        .register_asset_loader(PostcardLoader::<DotRes>::new("osdot"))
        .register_asset_loader(PostcardLoader::<VehicleRoutesRes>::new("osroutes"))
        .register_asset_loader(PostcardLoader::<NeighborhoodRes>::new("osneigh"))
        .register_asset_loader(PostcardLoader::<BusDayRes>::new("osbusday"))
        .register_asset_loader(PostcardLoader::<TaxiDayRes>::new("ostaxiday"))
        .register_asset_loader(PostcardLoader::<RobotabilityRes>::new("osrobot"))
        .register_asset_loader(PostcardLoader::<TeslaFieldRes>::new("osteslas"))
        .register_asset_loader(PostcardLoader::<BoroughRes>::new("osboro"))
        .register_asset_loader(PostcardLoader::<FootprintsRes>::new("osbldg"))
        .register_asset_loader(PostcardLoader::<ParksRes>::new("ospark"))
        .register_asset_loader(PostcardLoader::<PlazaRes>::new("osplaza"))
        .register_asset_loader(PostcardLoader::<LandmarkRes>::new("oslmk"))
        .register_asset_loader(PostcardLoader::<LinkNycRes>::new("oslink"));
}

/// Handles requested at startup; the world is built once they resolve.
#[derive(Resource)]
pub struct LoadingHandles {
    pub graph: Handle<GraphAssetRes>,
    /// Fixed-CCTV census (Amnesty + Dahir) with per-camera provenance (`.oscctv`).
    pub cameras: Handle<CctvRes>,
    /// Photo-enforcement cameras — a plain `FixedSensorLayer` (`.oscam`).
    pub enforcement: Handle<CamerasRes>,
    pub ace: Handle<AceRes>,
    pub heatmap: Handle<HeatmapRes>,
    pub equity: Handle<EquityRes>,
    pub dashcam: Handle<DashcamFieldRes>,
    pub alpr: Handle<AlprRes>,
    pub dot: Handle<DotRes>,
    pub vehicle_routes: Handle<VehicleRoutesRes>,
    pub neighborhoods: Handle<NeighborhoodRes>,
    pub bus_day: Handle<BusDayRes>,
    pub taxi_day: Handle<TaxiDayRes>,
    pub robotability: Handle<RobotabilityRes>,
    pub teslas: Handle<TeslaFieldRes>,
    pub borough: Handle<BoroughRes>,
    pub footprints: Handle<FootprintsRes>,
    /// Park polygons (green context fabric) — Manhattan-clipped or citywide.
    pub parks: Handle<ParksRes>,
    /// Pedestrian-plaza polygons (concrete fill + hatch); one asset for both builds.
    pub plazas: Handle<PlazaRes>,
    pub landmarks: Handle<LandmarkRes>,
    /// Iconic bridge massings (same `.oslmk` schema as `landmarks`).
    pub bridges: Handle<LandmarkRes>,
    pub linknyc: Handle<LinkNycRes>,
    pub built: bool,
}
