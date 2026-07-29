use waterui_map::{Coordinate, Region};

pub const TILE_SIZE: u32 = 512;
pub const TILE_OVERSCAN_PIXELS: u32 = TILE_SIZE / 2;
pub const MAX_CAMERA_ZOOM: u8 = 22;
const MAX_MERCATOR_LATITUDE: f64 = 85.051_128_779_806_6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileId {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub region: Region,
    pub viewport: Viewport,
    pub zoom: f64,
    pub tile_zoom: u8,
    center_world: (f64, f64),
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "validated finite camera and tile coordinates are intentionally quantized for GPU and MVT domains"
)]
impl Camera {
    pub fn new(region: Region, viewport: Viewport, min_zoom: u8, max_zoom: u8) -> Self {
        assert!(
            region.latitude_delta.is_finite() && region.latitude_delta > 0.0,
            "Map latitude_delta must be finite and positive"
        );
        assert!(
            region.longitude_delta.is_finite() && region.longitude_delta > 0.0,
            "Map longitude_delta must be finite and positive"
        );
        assert!(
            viewport.width > 0 && viewport.height > 0,
            "Map viewport must be non-zero"
        );

        let center = coordinate_world(region.center, 0.0);
        let north = latitude_world(
            region
                .latitude_delta
                .mul_add(0.5, region.center.latitude.get()),
            0.0,
        );
        let south = latitude_world(
            region
                .latitude_delta
                .mul_add(-0.5, region.center.latitude.get()),
            0.0,
        );
        let normalized_height = (south - north).abs().max(f64::EPSILON);
        let normalized_width = (region.longitude_delta / 360.0).max(f64::EPSILON);
        let zoom_x = (f64::from(viewport.width) / (f64::from(TILE_SIZE) * normalized_width)).log2();
        let zoom_y =
            (f64::from(viewport.height) / (f64::from(TILE_SIZE) * normalized_height)).log2();
        let zoom = zoom_x
            .min(zoom_y)
            .clamp(f64::from(min_zoom), f64::from(max_zoom));
        let tile_zoom = zoom.round().clamp(f64::from(min_zoom), f64::from(max_zoom)) as u8;
        let center_world = (center.0 * world_size(zoom), center.1 * world_size(zoom));
        Self {
            region,
            viewport,
            zoom,
            tile_zoom,
            center_world,
        }
    }

    pub fn visible_tiles(
        self,
        padding: f64,
        minimum_tile_zoom: u8,
        maximum_tile_zoom: u8,
    ) -> Vec<TileId> {
        let tile_zoom = self.tile_zoom.clamp(minimum_tile_zoom, maximum_tile_zoom);
        let scale = (self.zoom - f64::from(tile_zoom)).exp2();
        let tile_screen_size = f64::from(TILE_SIZE) * scale;
        let center_tile = (
            coordinate_world(self.region.center, f64::from(tile_zoom)).0
                * 2.0_f64.powi(i32::from(tile_zoom)),
            coordinate_world(self.region.center, f64::from(tile_zoom)).1
                * 2.0_f64.powi(i32::from(tile_zoom)),
        );
        let half_x = f64::from(self.viewport.width).mul_add(0.5, padding) / tile_screen_size;
        let half_y = f64::from(self.viewport.height).mul_add(0.5, padding) / tile_screen_size;
        let min_x = (center_tile.0 - half_x).floor() as i64;
        let max_x = (center_tile.0 + half_x).floor() as i64;
        let min_y = (center_tile.1 - half_y).floor() as i64;
        let max_y = (center_tile.1 + half_y).floor() as i64;
        let dimension = 1_i64 << tile_zoom;
        let mut tiles = Vec::new();
        for y in min_y.max(0)..=max_y.min(dimension - 1) {
            for x in min_x..=max_x {
                let wrapped_x = x.rem_euclid(dimension);
                tiles.push(TileId {
                    z: tile_zoom,
                    x: u32::try_from(wrapped_x).expect("wrapped tile x must fit u32"),
                    y: u32::try_from(y).expect("clamped tile y must fit u32"),
                });
            }
        }
        tiles
    }

    pub fn tile_point(self, tile: TileId, extent: u32, x: f32, y: f32) -> (f64, f64) {
        let tile_scale = (self.zoom - f64::from(tile.z)).exp2();
        let world_x = f64::from(tile.x).mul_add(
            f64::from(TILE_SIZE),
            f64::from(x) * f64::from(TILE_SIZE) / f64::from(extent),
        ) * tile_scale;
        let world_y = f64::from(tile.y).mul_add(
            f64::from(TILE_SIZE),
            f64::from(y) * f64::from(TILE_SIZE) / f64::from(extent),
        ) * tile_scale;
        (
            f64::from(self.viewport.width).mul_add(0.5, world_x - self.center_world.0),
            f64::from(self.viewport.height).mul_add(0.5, world_y - self.center_world.1),
        )
    }

    pub fn tile_units_per_pixel(self, tile_zoom: u8, extent: u32) -> f32 {
        let scale = (self.zoom - f64::from(tile_zoom)).exp2();
        (f64::from(extent) / (f64::from(TILE_SIZE) * scale)) as f32
    }

    pub fn coordinate_point(self, coordinate: Coordinate) -> (f64, f64) {
        let normalized = coordinate_world(coordinate, self.zoom);
        let world = world_size(self.zoom);
        (
            f64::from(self.viewport.width)
                .mul_add(0.5, normalized.0.mul_add(world, -self.center_world.0)),
            f64::from(self.viewport.height)
                .mul_add(0.5, normalized.1.mul_add(world, -self.center_world.1)),
        )
    }

    pub fn meters_to_pixels(self, latitude: f64, meters: f64) -> f64 {
        let earth_circumference = 40_075_016.686;
        meters * world_size(self.zoom)
            / (earth_circumference * latitude.to_radians().cos().abs().max(0.01))
    }
}

fn world_size(zoom: f64) -> f64 {
    f64::from(TILE_SIZE) * zoom.exp2()
}

fn coordinate_world(coordinate: Coordinate, _zoom: f64) -> (f64, f64) {
    (
        (coordinate.longitude.get() + 180.0) / 360.0,
        latitude_world(coordinate.latitude.get(), 0.0),
    )
}

fn latitude_world(latitude: f64, _zoom: f64) -> f64 {
    let latitude = latitude.clamp(-MAX_MERCATOR_LATITUDE, MAX_MERCATOR_LATITUDE);
    let sine = latitude.to_radians().sin();
    0.5 - ((1.0 + sine) / (1.0 - sine)).ln() / (4.0 * std::f64::consts::PI)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_overzooms_a_vector_source_without_requesting_missing_tiles() {
        let region = Region::new(
            Coordinate::from_degrees(40.7580, -73.9855)
                .expect("Manhattan test coordinate must be valid"),
            0.003,
            0.005,
        );
        let camera = Camera::new(
            region,
            Viewport {
                width: 1_600,
                height: 1_200,
            },
            0,
            MAX_CAMERA_ZOOM,
        );

        assert!(camera.zoom > 14.0);
        assert!(camera.tile_zoom > 14);
        let tiles = camera.visible_tiles(f64::from(TILE_OVERSCAN_PIXELS), 0, 14);
        assert!(!tiles.is_empty());
        assert!(tiles.iter().all(|tile| tile.z == 14));
    }
}
