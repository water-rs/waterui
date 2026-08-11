use std::collections::BTreeMap;

use futures::future::join_all;
use maplibre_expr::{Expr, Type, parse, typecheck};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;
use url::Url;

use crate::{MapGpuOptions, MapLoadError, network};

#[derive(Debug, Deserialize)]
struct RawStyle {
    version: u8,
    #[serde(default)]
    sources: BTreeMap<String, RawSource>,
    layers: Vec<RawLayer>,
}

#[derive(Debug, Deserialize)]
struct RawSource {
    #[serde(rename = "type")]
    kind: String,
    url: Option<String>,
    tiles: Option<Vec<String>>,
    minzoom: Option<u8>,
    maxzoom: Option<u8>,
    #[serde(rename = "tileSize")]
    tile_size: Option<u32>,
    encoding: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawLayer {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    source: Option<String>,
    #[serde(rename = "source-layer")]
    source_layer: Option<String>,
    minzoom: Option<f64>,
    maxzoom: Option<f64>,
    filter: Option<Value>,
    #[serde(default)]
    paint: BTreeMap<String, Value>,
    #[serde(default)]
    layout: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
struct TileJson {
    tiles: Vec<String>,
    minzoom: Option<u8>,
    maxzoom: Option<u8>,
    #[serde(rename = "tileSize")]
    tile_size: Option<u32>,
    encoding: Option<String>,
}

/// The tile payload a source serves, which decides how its tiles decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// Mapbox Vector Tile geometry.
    Vector,
    /// Pre-rendered image tiles.
    Raster,
    /// Image tiles whose channels encode terrain elevation.
    RasterDem,
}

/// How a `raster-dem` source packs elevation into its colour channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DemEncoding {
    /// `height = -10000 + (R * 256 * 256 + G * 256 + B) * 0.1`
    #[default]
    Mapbox,
    /// `height = (R * 256 + G + B / 256) - 32768`
    Terrarium,
}

impl DemEncoding {
    /// Decodes one DEM texel into metres above sea level.
    pub fn height(self, red: u8, green: u8, blue: u8) -> f32 {
        let (red, green, blue) = (f32::from(red), f32::from(green), f32::from(blue));
        match self {
            Self::Mapbox => blue.mul_add(0.1, red.mul_add(6553.6, green * 25.6)) - 10_000.0,
            Self::Terrarium => (blue / 256.0) + red.mul_add(256.0, green) - 32_768.0,
        }
    }
}

/// The pixel size assumed for a raster tile when a provider omits `tileSize`.
const DEFAULT_RASTER_TILE_SIZE: u32 = 256;

#[derive(Debug, Clone)]
pub struct TileSource {
    pub kind: SourceKind,
    pub templates: Vec<String>,
    pub min_zoom: u8,
    pub max_zoom: u8,
    pub tile_size: u32,
    pub encoding: DemEncoding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    Background,
    Fill,
    FillExtrusion,
    Line,
    Symbol,
    Circle,
    Heatmap,
    Hillshade,
    Raster,
    Sky,
}

impl LayerKind {
    /// Returns the source payload this layer draws from, or `None` for the
    /// source-less kinds (`background`, `sky`).
    pub const fn required_source(self) -> Option<SourceKind> {
        match self {
            Self::Background | Self::Sky => None,
            Self::Fill
            | Self::FillExtrusion
            | Self::Line
            | Self::Symbol
            | Self::Circle
            | Self::Heatmap => Some(SourceKind::Vector),
            Self::Raster => Some(SourceKind::Raster),
            Self::Hillshade => Some(SourceKind::RasterDem),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StyleLayer {
    pub id: String,
    pub kind: LayerKind,
    pub source: Option<String>,
    pub source_layer: Option<String>,
    pub min_zoom: f64,
    pub max_zoom: f64,
    pub filter: Option<Expr>,
    pub properties: BTreeMap<String, Expr>,
    /// Resolved `layout.visibility`. The spec makes this a constant, so it is
    /// settled at load time rather than evaluated per feature.
    pub visible: bool,
}

impl StyleLayer {
    pub fn active(&self, zoom: f64) -> bool {
        self.visible && zoom >= self.min_zoom && zoom < self.max_zoom
    }

    pub fn property(&self, name: &str) -> Option<&Expr> {
        self.properties.get(name)
    }
}

#[derive(Debug, Clone)]
pub struct MapStyle {
    pub sources: BTreeMap<String, TileSource>,
    pub layers: Vec<StyleLayer>,
}

impl MapStyle {
    #[allow(
        clippy::future_not_send,
        reason = "map style loading runs on WaterUI's main-thread local executor"
    )]
    pub async fn load(
        options: &MapGpuOptions,
        style: waterui_map::MapStyle,
    ) -> Result<Self, MapLoadError> {
        let configured = options.style_url(style)?;
        let style_url =
            Url::parse(configured.as_ref()).map_err(|source| MapLoadError::InvalidUrl {
                url: configured.to_string(),
                source,
            })?;
        let bytes = network::fetch(
            style_url.as_str(),
            options.maximum_style_bytes.get(),
            options.network_request_timeout(),
        )
        .await?;
        let style_url_text = style_url.to_string();
        let raw: RawStyle = blocking::unblock(move || parse_json(style_url_text, &bytes)).await?;
        if raw.version != 8 {
            return Err(MapLoadError::Unsupported(format!(
                "MapLibre style version {} (expected 8)",
                raw.version
            )));
        }

        let source_requests = raw
            .sources
            .into_iter()
            .filter(|(_, source)| source_kind(&source.kind).is_some())
            .map(|(name, source)| {
                let style_url = style_url.clone();
                async move {
                    resolve_source(&style_url, source, options)
                        .await
                        .map(|source| (name, source))
                }
            });
        let sources = async {
            join_all(source_requests)
                .await
                .into_iter()
                .collect::<Result<BTreeMap<_, _>, _>>()
        };
        let layers = blocking::unblock(move || {
            raw.layers
                .into_iter()
                .map(compile_layer)
                .collect::<Result<Vec<_>, _>>()
        });
        let (sources, layers) = futures::try_join!(sources, layers)?;
        let style = Self { sources, layers };
        style.validate_layer_sources()?;
        Ok(style)
    }

    /// Rejects layers whose source is missing or serves the wrong payload.
    ///
    /// This is checked once at load time so the painter can rely on every
    /// active layer having a source it can actually draw, instead of
    /// discovering the mismatch inside the render loop.
    fn validate_layer_sources(&self) -> Result<(), MapLoadError> {
        for layer in &self.layers {
            let Some(required) = layer.kind.required_source() else {
                continue;
            };
            let name = layer.source.as_ref().ok_or_else(|| {
                MapLoadError::Unsupported(format!("style layer {} requires a source", layer.id))
            })?;
            let source = self.sources.get(name).ok_or_else(|| {
                MapLoadError::Unsupported(format!(
                    "style layer {} references undefined source {name}",
                    layer.id
                ))
            })?;
            if source.kind != required {
                return Err(MapLoadError::Unsupported(format!(
                    "style layer {} requires a {required:?} source but {name} is {:?}",
                    layer.id, source.kind
                )));
            }
        }
        Ok(())
    }

    pub const fn camera_zoom_range() -> (u8, u8) {
        (0, crate::projection::MAX_CAMERA_ZOOM)
    }
}

#[allow(
    clippy::future_not_send,
    reason = "map source resolution runs on WaterUI's main-thread local executor"
)]
async fn resolve_source(
    style_url: &Url,
    source: RawSource,
    options: &MapGpuOptions,
) -> Result<TileSource, MapLoadError> {
    let kind = source_kind(&source.kind).ok_or_else(|| {
        MapLoadError::Unsupported(format!("source type {} is not a tile source", source.kind))
    })?;
    let default_tile_size = match kind {
        SourceKind::Vector => crate::projection::TILE_SIZE,
        SourceKind::Raster | SourceKind::RasterDem => DEFAULT_RASTER_TILE_SIZE,
    };
    if let Some(tiles) = source.tiles {
        return Ok(TileSource {
            kind,
            templates: resolve_templates(style_url, tiles)?,
            min_zoom: source.minzoom.unwrap_or(0),
            max_zoom: source.maxzoom.unwrap_or(22),
            tile_size: source.tile_size.unwrap_or(default_tile_size),
            encoding: dem_encoding(source.encoding.as_deref())?,
        });
    }
    let source_url = source.url.ok_or_else(|| {
        MapLoadError::Unsupported(String::from("tile source requires either url or tiles"))
    })?;
    let tilejson_url = style_url
        .join(&source_url)
        .map_err(|source| MapLoadError::InvalidUrl {
            url: source_url,
            source,
        })?;
    let bytes = network::fetch(
        tilejson_url.as_str(),
        options.maximum_tilejson_bytes.get(),
        options.network_request_timeout(),
    )
    .await?;
    let tilejson_url_text = tilejson_url.to_string();
    let tilejson: TileJson =
        blocking::unblock(move || parse_json(tilejson_url_text, &bytes)).await?;
    Ok(TileSource {
        kind,
        templates: resolve_templates(&tilejson_url, tilejson.tiles)?,
        min_zoom: source.minzoom.or(tilejson.minzoom).unwrap_or(0),
        max_zoom: source.maxzoom.or(tilejson.maxzoom).unwrap_or(22),
        tile_size: source
            .tile_size
            .or(tilejson.tile_size)
            .unwrap_or(default_tile_size),
        encoding: dem_encoding(source.encoding.as_deref().or(tilejson.encoding.as_deref()))?,
    })
}

const fn source_kind(kind: &str) -> Option<SourceKind> {
    // `const fn` cannot match on `&str`, so compare the bytes directly.
    match kind.as_bytes() {
        b"vector" => Some(SourceKind::Vector),
        b"raster" => Some(SourceKind::Raster),
        b"raster-dem" => Some(SourceKind::RasterDem),
        _ => None,
    }
}

fn dem_encoding(encoding: Option<&str>) -> Result<DemEncoding, MapLoadError> {
    match encoding {
        None | Some("mapbox") => Ok(DemEncoding::Mapbox),
        Some("terrarium") => Ok(DemEncoding::Terrarium),
        Some(other) => Err(MapLoadError::Unsupported(format!(
            "raster-dem encoding {other} is not supported"
        ))),
    }
}

fn parse_json<T: DeserializeOwned>(url: String, bytes: &[u8]) -> Result<T, MapLoadError> {
    serde_json::from_slice(bytes).map_err(|source| MapLoadError::Json { url, source })
}

fn resolve_templates(base: &Url, templates: Vec<String>) -> Result<Vec<String>, MapLoadError> {
    templates
        .into_iter()
        .map(|template| {
            base.join(&template)
                .map(|url| {
                    url.to_string()
                        .replace("%7Bz%7D", "{z}")
                        .replace("%7Bx%7D", "{x}")
                        .replace("%7By%7D", "{y}")
                })
                .map_err(|source| MapLoadError::InvalidUrl {
                    url: template,
                    source,
                })
        })
        .collect()
}

fn compile_layer(raw: RawLayer) -> Result<StyleLayer, MapLoadError> {
    let kind = match raw.kind.as_str() {
        "background" => LayerKind::Background,
        "fill" => LayerKind::Fill,
        "fill-extrusion" => LayerKind::FillExtrusion,
        "line" => LayerKind::Line,
        "symbol" => LayerKind::Symbol,
        "circle" => LayerKind::Circle,
        "heatmap" => LayerKind::Heatmap,
        "hillshade" => LayerKind::Hillshade,
        "raster" => LayerKind::Raster,
        "sky" => LayerKind::Sky,
        other => {
            return Err(MapLoadError::Unsupported(format!(
                "style layer {} has unsupported type {other}",
                raw.id
            )));
        }
    };
    let filter = raw
        .filter
        .as_ref()
        .map(|value| compile_expression(&raw.id, "filter", value, Some(Type::Boolean), false))
        .transpose()?;
    let visible = layer_visibility(&raw.id, raw.layout.get("visibility"))?;
    let mut properties = BTreeMap::new();
    for (name, value) in raw.paint.into_iter().chain(raw.layout) {
        let expected = expected_property_type(&name);
        if let Some((expected, coerce_string)) = expected {
            let expression =
                compile_expression(&raw.id, &name, &value, Some(expected), coerce_string)?;
            properties.insert(name, expression);
        }
    }
    Ok(StyleLayer {
        id: raw.id,
        kind,
        source: raw.source,
        source_layer: raw.source_layer,
        min_zoom: raw.minzoom.unwrap_or(f64::NEG_INFINITY),
        max_zoom: raw.maxzoom.unwrap_or(f64::INFINITY),
        filter,
        properties,
        visible,
    })
}

/// `layout.visibility` is a constant enum in the style spec, so it resolves at
/// load time. Anything other than the two spec values is a malformed style.
fn layer_visibility(layer: &str, value: Option<&Value>) -> Result<bool, MapLoadError> {
    match value {
        None => Ok(true),
        Some(Value::String(visibility)) if visibility == "visible" => Ok(true),
        Some(Value::String(visibility)) if visibility == "none" => Ok(false),
        Some(other) => Err(MapLoadError::Expression {
            layer: layer.to_owned(),
            property: String::from("visibility"),
            message: format!("expected \"visible\" or \"none\", got {other}"),
        }),
    }
}

fn expected_property_type(name: &str) -> Option<(Type, bool)> {
    if name.ends_with("-color") {
        Some((Type::Color, false))
    } else if matches!(
        name,
        "fill-opacity"
            | "fill-extrusion-opacity"
            | "line-opacity"
            | "line-width"
            | "line-gap-width"
            | "text-size"
            | "text-padding"
            | "text-halo-width"
            | "text-halo-blur"
            | "circle-radius"
            | "circle-opacity"
            | "circle-blur"
            | "circle-stroke-width"
            | "circle-stroke-opacity"
            | "heatmap-radius"
            | "heatmap-weight"
            | "heatmap-intensity"
            | "heatmap-opacity"
            | "hillshade-exaggeration"
            | "raster-opacity"
            | "raster-brightness-min"
            | "raster-brightness-max"
            | "raster-saturation"
            | "raster-contrast"
            | "sky-opacity"
            | "sky-atmosphere-sun-intensity"
    ) {
        Some((Type::Number, false))
    } else if matches!(
        name,
        "text-field" | "symbol-placement" | "text-anchor" | "sky-type"
    ) {
        Some((Type::String, true))
    } else if matches!(name, "text-offset" | "sky-atmosphere-sun") {
        Some((Type::array(Type::Number, Some(2)), false))
    } else {
        None
    }
}

fn compile_expression(
    layer: &str,
    property: &str,
    value: &Value,
    expected: Option<Type>,
    coerce_string: bool,
) -> Result<Expr, MapLoadError> {
    let literal = matches!(expected, Some(Type::Array(..)))
        .then(|| value.as_array())
        .flatten()
        .filter(|values| !matches!(values.first(), Some(Value::String(_))))
        .map(|_| Value::Array(vec![Value::String("literal".to_owned()), value.to_owned()]));
    let expression = literal.as_ref().unwrap_or(value);
    let parsed = parse(expression).map_err(|error| MapLoadError::Expression {
        layer: layer.to_owned(),
        property: property.to_owned(),
        message: error.to_string(),
    })?;
    expected.map_or_else(
        || Ok(parsed.clone()),
        |expected| {
            typecheck(&parsed, Some(&expected), coerce_string).map_err(|error| {
                MapLoadError::Expression {
                    layer: layer.to_owned(),
                    property: property.to_owned(),
                    message: error.to_string(),
                }
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(id: &str, kind: LayerKind, source: Option<&str>) -> StyleLayer {
        StyleLayer {
            id: id.to_owned(),
            kind,
            source: source.map(str::to_owned),
            source_layer: None,
            min_zoom: f64::NEG_INFINITY,
            max_zoom: f64::INFINITY,
            filter: None,
            properties: BTreeMap::new(),
            visible: true,
        }
    }

    fn source(kind: SourceKind) -> TileSource {
        TileSource {
            kind,
            templates: vec![String::from("https://tiles.example/{z}/{x}/{y}")],
            min_zoom: 0,
            max_zoom: 14,
            tile_size: DEFAULT_RASTER_TILE_SIZE,
            encoding: DemEncoding::Mapbox,
        }
    }

    /// A raster layer bound to a vector source used to reach the painter and
    /// panic mid-frame; the mismatch is now a load-time error.
    #[test]
    fn a_layer_bound_to_the_wrong_source_payload_fails_to_load() {
        let style = MapStyle {
            sources: BTreeMap::from([(String::from("basemap"), source(SourceKind::Vector))]),
            layers: vec![layer("satellite", LayerKind::Raster, Some("basemap"))],
        };

        let error = style
            .validate_layer_sources()
            .expect_err("a raster layer over a vector source must be rejected");

        assert!(
            matches!(error, MapLoadError::Unsupported(message) if message.contains("satellite")),
            "the error must name the offending layer"
        );
    }

    #[test]
    fn a_layer_referencing_an_undefined_source_fails_to_load() {
        let style = MapStyle {
            sources: BTreeMap::new(),
            layers: vec![layer("roads", LayerKind::Line, Some("missing"))],
        };

        assert!(style.validate_layer_sources().is_err());
    }

    #[test]
    fn source_less_layer_kinds_need_no_source() {
        let style = MapStyle {
            sources: BTreeMap::new(),
            layers: vec![
                layer("bg", LayerKind::Background, None),
                layer("sky", LayerKind::Sky, None),
            ],
        };

        assert!(style.validate_layer_sources().is_ok());
    }

    #[test]
    fn hidden_layers_are_inactive_at_every_zoom() {
        let mut hidden = layer("labels", LayerKind::Symbol, Some("basemap"));
        hidden.visible = false;

        assert!(!hidden.active(0.0));
        assert!(!hidden.active(14.0));
    }

    #[test]
    fn visibility_resolves_from_the_layout_property() {
        assert!(layer_visibility("a", None).expect("absent visibility defaults to visible"));
        assert!(
            layer_visibility("a", Some(&Value::String(String::from("visible"))))
                .expect("explicit visible")
        );
        assert!(
            !layer_visibility("a", Some(&Value::String(String::from("none"))))
                .expect("explicit none")
        );
        assert!(layer_visibility("a", Some(&Value::Bool(true))).is_err());
    }

    #[test]
    fn mapbox_and_terrarium_dem_encodings_decode_known_elevations() {
        // Mapbox: sea level is exactly (1, 134, 160).
        assert!((DemEncoding::Mapbox.height(1, 134, 160) - 0.0).abs() < 0.05);
        // Terrarium: sea level is (128, 0, 0).
        assert!((DemEncoding::Terrarium.height(128, 0, 0) - 0.0).abs() < 0.05);
        assert!(DemEncoding::Mapbox.height(1, 200, 0) > 0.0);
        assert!(DemEncoding::Terrarium.height(127, 0, 0) < 0.0);
    }

    #[test]
    fn raster_dem_encoding_is_rejected_when_unrecognized() {
        assert!(dem_encoding(Some("float32")).is_err());
        assert_eq!(
            dem_encoding(None).expect("absent encoding defaults to mapbox"),
            DemEncoding::Mapbox
        );
    }
}
