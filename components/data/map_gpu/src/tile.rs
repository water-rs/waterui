use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use geo_types::Geometry;
use maplibre_expr::{Feature as StyleFeature, Value as StyleValue};
use mvt_reader::{Reader, feature};

use crate::{MapLoadError, projection::TileId, style::DemEncoding};

#[derive(Debug, Clone)]
pub struct TileFeature {
    pub geometry: Geometry<f32>,
    pub style: StyleFeature,
}

#[derive(Debug, Clone)]
pub struct TileLayer {
    pub extent: u32,
    pub features: Vec<TileFeature>,
}

#[derive(Debug, Clone)]
pub struct VectorTile {
    pub id: TileId,
    pub layers: HashMap<String, TileLayer>,
    pub byte_len: usize,
}

/// A decoded `raster` tile held as RGBA8.
///
/// The pixels live behind `Arc<Vec<u8>>` rather than `Arc<[u8]>` so they can be
/// handed to `peniko::Blob` without another copy per frame.
#[derive(Debug, Clone)]
pub struct RasterTile {
    pub id: TileId,
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<Vec<u8>>,
    pub byte_len: usize,
}

impl RasterTile {
    pub fn decode(id: TileId, bytes: &[u8]) -> Result<Self, MapLoadError> {
        let image = decode_image(id, bytes)?.into_rgba8();
        let (width, height) = image.dimensions();
        let pixels = image.into_raw();
        Ok(Self {
            id,
            width,
            height,
            byte_len: pixels.len(),
            pixels: Arc::new(pixels),
        })
    }
}

/// A decoded `raster-dem` tile resolved to metres above sea level.
#[derive(Debug, Clone)]
pub struct DemTile {
    pub id: TileId,
    pub width: u32,
    pub height: u32,
    pub heights: Arc<[f32]>,
    pub byte_len: usize,
}

impl DemTile {
    pub fn decode(id: TileId, bytes: &[u8], encoding: DemEncoding) -> Result<Self, MapLoadError> {
        let image = decode_image(id, bytes)?.into_rgb8();
        let (width, height) = image.dimensions();
        let heights: Arc<[f32]> = image
            .pixels()
            .map(|pixel| encoding.height(pixel.0[0], pixel.0[1], pixel.0[2]))
            .collect();
        Ok(Self {
            id,
            width,
            height,
            byte_len: heights.len() * size_of::<f32>(),
            heights,
        })
    }

    /// Returns the elevation at a clamped texel coordinate.
    pub fn height_at(&self, x: i64, y: i64) -> f32 {
        let x = x.clamp(0, i64::from(self.width) - 1);
        let y = y.clamp(0, i64::from(self.height) - 1);
        let index = y * i64::from(self.width) + x;
        self.heights[usize::try_from(index).expect("clamped DEM index must be in range")]
    }
}

fn decode_image(id: TileId, bytes: &[u8]) -> Result<image::DynamicImage, MapLoadError> {
    image::load_from_memory(bytes).map_err(|error| MapLoadError::ImageTile {
        z: id.z,
        x: id.x,
        y: id.y,
        message: error.to_string(),
    })
}

impl VectorTile {
    pub fn decode(id: TileId, bytes: Vec<u8>) -> Result<Self, MapLoadError> {
        let byte_len = bytes.len();
        let reader = Reader::new(bytes).map_err(|error| MapLoadError::VectorTile {
            z: id.z,
            x: id.x,
            y: id.y,
            message: error.to_string(),
        })?;
        let metadata = reader
            .get_layer_metadata()
            .map_err(|error| MapLoadError::VectorTile {
                z: id.z,
                x: id.x,
                y: id.y,
                message: error.to_string(),
            })?;
        let mut layers = HashMap::with_capacity(metadata.len());
        for layer in metadata {
            let features = reader.get_features(layer.layer_index).map_err(|error| {
                MapLoadError::VectorTile {
                    z: id.z,
                    x: id.x,
                    y: id.y,
                    message: error.to_string(),
                }
            })?;
            layers.insert(
                layer.name,
                TileLayer {
                    extent: layer.extent,
                    features: features
                        .into_iter()
                        .map(|feature| TileFeature {
                            style: style_feature(&feature),
                            geometry: feature.geometry,
                        })
                        .collect(),
                },
            );
        }
        Ok(Self {
            id,
            layers,
            byte_len,
        })
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "MapLibre expressions use f64 numbers, matching JavaScript number semantics for MVT integer properties"
)]
fn style_feature(feature: &feature::Feature) -> StyleFeature {
    let properties = feature
        .properties
        .as_ref()
        .map(|properties| {
            properties
                .iter()
                .map(|(key, value)| (key.clone(), style_value(value)))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    StyleFeature {
        id: feature.id.map(|id| StyleValue::Number(id as f64)),
        properties,
        geometry_type: Some(geometry_type(&feature.geometry).to_owned()),
        state: BTreeMap::new(),
        geometry: Vec::new(),
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "MapLibre expressions use f64 numbers, matching JavaScript number semantics for MVT integer properties"
)]
fn style_value(value: &feature::Value) -> StyleValue {
    match value {
        feature::Value::String(value) => StyleValue::String(value.clone()),
        feature::Value::Float(value) => StyleValue::Number(f64::from(*value)),
        feature::Value::Double(value) => StyleValue::Number(*value),
        feature::Value::Int(value) | feature::Value::SInt(value) => {
            StyleValue::Number(*value as f64)
        }
        feature::Value::UInt(value) => StyleValue::Number(*value as f64),
        feature::Value::Bool(value) => StyleValue::Bool(*value),
        feature::Value::Null => StyleValue::Null,
    }
}

const fn geometry_type(geometry: &Geometry<f32>) -> &'static str {
    match geometry {
        Geometry::Point(_) | Geometry::MultiPoint(_) => "Point",
        Geometry::Line(_) | Geometry::LineString(_) | Geometry::MultiLineString(_) => "LineString",
        Geometry::Polygon(_)
        | Geometry::MultiPolygon(_)
        | Geometry::Rect(_)
        | Geometry::Triangle(_) => "Polygon",
        Geometry::GeometryCollection(_) => "Unknown",
    }
}
