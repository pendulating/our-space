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
    AceCorridorLayer, DashcamFieldLayer, EquityLayer, FixedSensorLayer, GraphAsset, HeatmapLayer,
};

macro_rules! postcard_asset {
    ($name:ident, $inner:ty) => {
        #[derive(Asset, TypePath, serde::Deserialize)]
        pub struct $name(pub $inner);
    };
}

postcard_asset!(GraphAssetRes, GraphAsset);
postcard_asset!(CamerasRes, FixedSensorLayer);
postcard_asset!(AceRes, AceCorridorLayer);
postcard_asset!(HeatmapRes, HeatmapLayer);
postcard_asset!(EquityRes, EquityLayer);
postcard_asset!(DashcamFieldRes, DashcamFieldLayer);
postcard_asset!(AlprRes, FixedSensorLayer);
postcard_asset!(DotRes, FixedSensorLayer);

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
        .init_asset::<AceRes>()
        .init_asset::<HeatmapRes>()
        .init_asset::<EquityRes>()
        .init_asset::<DashcamFieldRes>()
        .init_asset::<AlprRes>()
        .init_asset::<DotRes>()
        .register_asset_loader(PostcardLoader::<GraphAssetRes>::new("osgraph"))
        .register_asset_loader(PostcardLoader::<CamerasRes>::new("oscam"))
        .register_asset_loader(PostcardLoader::<AceRes>::new("osace"))
        .register_asset_loader(PostcardLoader::<HeatmapRes>::new("osheat"))
        .register_asset_loader(PostcardLoader::<EquityRes>::new("osequity"))
        .register_asset_loader(PostcardLoader::<DashcamFieldRes>::new("osfield"))
        .register_asset_loader(PostcardLoader::<AlprRes>::new("osalpr"))
        .register_asset_loader(PostcardLoader::<DotRes>::new("osdot"));
}

/// Handles requested at startup; the world is built once they resolve.
#[derive(Resource)]
pub struct LoadingHandles {
    pub graph: Handle<GraphAssetRes>,
    pub cameras: Handle<CamerasRes>,
    pub ace: Handle<AceRes>,
    pub heatmap: Handle<HeatmapRes>,
    pub equity: Handle<EquityRes>,
    pub dashcam: Handle<DashcamFieldRes>,
    pub alpr: Handle<AlprRes>,
    pub dot: Handle<DotRes>,
    pub built: bool,
}
