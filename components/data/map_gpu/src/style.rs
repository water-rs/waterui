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
}

#[derive(Debug, Clone)]
pub struct VectorSource {
    pub templates: Vec<String>,
    pub min_zoom: u8,
    pub max_zoom: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    Background,
    Fill,
    FillExtrusion,
    Line,
    Symbol,
    Raster,
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
}

impl StyleLayer {
    pub fn active(&self, zoom: f64) -> bool {
        zoom >= self.min_zoom && zoom < self.max_zoom
    }

    pub fn property(&self, name: &str) -> Option<&Expr> {
        self.properties.get(name)
    }
}

#[derive(Debug, Clone)]
pub struct MapStyle {
    pub sources: BTreeMap<String, VectorSource>,
    pub layers: Vec<StyleLayer>,
}

impl MapStyle {
    #[allow(
        clippy::future_not_send,
        reason = "map style loading runs on WaterUI's main-thread local executor"
    )]
    pub async fn load(options: &MapGpuOptions) -> Result<Self, MapLoadError> {
        let style_url = Url::parse(options.style_url().as_ref()).map_err(|source| {
            MapLoadError::InvalidUrl {
                url: options.style_url().to_string(),
                source,
            }
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
            .filter(|(_, source)| source.kind == "vector")
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
        Ok(Self { sources, layers })
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
) -> Result<VectorSource, MapLoadError> {
    if let Some(tiles) = source.tiles {
        return Ok(VectorSource {
            templates: resolve_templates(style_url, tiles)?,
            min_zoom: source.minzoom.unwrap_or(0),
            max_zoom: source.maxzoom.unwrap_or(22),
        });
    }
    let source_url = source.url.ok_or_else(|| {
        MapLoadError::Unsupported(String::from("vector source requires either url or tiles"))
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
    Ok(VectorSource {
        templates: resolve_templates(&tilejson_url, tilejson.tiles)?,
        min_zoom: source.minzoom.or(tilejson.minzoom).unwrap_or(0),
        max_zoom: source.maxzoom.or(tilejson.maxzoom).unwrap_or(22),
    })
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
        "raster" => LayerKind::Raster,
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
    })
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
    ) {
        Some((Type::Number, false))
    } else if matches!(name, "text-field" | "symbol-placement" | "text-anchor") {
        Some((Type::String, true))
    } else if name == "text-offset" {
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
