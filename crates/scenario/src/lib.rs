//! The Rocca Ventosa territory, and the coordinate frames that tie
//! them together.
//!
//! Three resolutions coexist deliberately:
//!
//! - the **fire grid**, 20 m, fixed by the PROPAGATOR input rasters;
//! - the **render terrain**, currently 5 m, purely a visual concern;
//! - **vector geometry and agents**, unquantised metres.
//!
//! Everything is expressed in one *world frame*: origin at the scenario
//! window's south-west corner, +x east, +y north, metres. Converting to fire
//! cells is [`World::cell_of`]; nothing else in the game should need to know
//! the raster spacing exists.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

pub mod cover;
pub mod fuels;
pub mod grade;
pub mod metadata;
pub mod population;
pub mod terrain;
pub mod vectors;

pub use cover::{Cover, CoverClass};
pub use fuels::FuelDefRaw;
pub use metadata::ScenarioMetadata;
pub use population::{Dwelling, Household, Person, Population};
pub use terrain::Terrain;
pub use vectors::{Building, Road, Vectors, WaterSource};

/// A cell index into the 20 m fire grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cell {
    pub row: usize,
    pub col: usize,
}

/// Metric position in the scenario world frame.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Pos {
    pub x: f32,
    pub y: f32,
}

impl From<[f32; 2]> for Pos {
    fn from(v: [f32; 2]) -> Self {
        Pos { x: v[0], y: v[1] }
    }
}

/// Geometry shared by every layer: how big the world is and how it maps onto
/// the fire raster.
#[derive(Debug, Clone, Copy)]
pub struct World {
    pub width_m: f32,
    pub height_m: f32,
    pub fire_rows: usize,
    pub fire_cols: usize,
    pub cellsize: f32,
}

impl World {
    /// World metres -> fire grid cell. Note the row flip: the world frame has
    /// +y north, the raster has row 0 at the north edge.
    pub fn cell_of(&self, p: Pos) -> Cell {
        let col = (p.x / self.cellsize).floor().clamp(0.0, (self.fire_cols - 1) as f32);
        let row = ((self.height_m - p.y) / self.cellsize)
            .floor()
            .clamp(0.0, (self.fire_rows - 1) as f32);
        Cell { row: row as usize, col: col as usize }
    }

    /// Centre of a fire cell, in world metres.
    pub fn centre_of(&self, c: Cell) -> Pos {
        Pos {
            x: (c.col as f32 + 0.5) * self.cellsize,
            y: self.height_m - (c.row as f32 + 0.5) * self.cellsize,
        }
    }

    pub fn contains(&self, p: Pos) -> bool {
        p.x >= 0.0 && p.y >= 0.0 && p.x < self.width_m && p.y < self.height_m
    }
}

/// Everything needed to start a scenario.
pub struct Scenario {
    pub id: String,
    pub metadata: ScenarioMetadata,
    pub world: World,
    pub terrain: Terrain,
    pub vectors: Vectors,
    pub population: Population,
    /// 20 m fuel classes, row-major, row 0 = north. `eu_fuel12` coding.
    pub fuel: Vec<i32>,
    /// 20 m elevation, matching `fuel`.
    pub dem: Vec<f64>,
    /// The eu_fuel12 class table these rasters are coded against.
    pub fuel_defs: Vec<FuelDefRaw>,
    /// 5 m land cover for drawing the ground, if the scenario has one.
    pub cover: Option<Cover>,
}

/// The one territory the game is played on (`data/scenarios/rocca_ventosa`,
/// published by `tools/scenario_factory.py publish`).
pub const ID: &str = "rocca_ventosa";

/// The territory to play: `ROCCA_SCENARIO` if set (a territory under trial),
/// else [`ID`].
pub fn id() -> String {
    std::env::var("ROCCA_SCENARIO").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| ID.to_string())
}

impl Scenario {
    /// Load the game's territory from a data directory (`data`).
    pub fn load(data_dir: impl AsRef<Path>) -> Result<Scenario> {
        Self::load_by_id(data_dir, id())
    }

    /// Load a scenario directory `data_dir/scenarios/<id>/`. The game has one
    /// ([`ID`]); the Scenario Factory uses this for the towns it is building.
    pub fn load_by_id(data_dir: impl AsRef<Path>, id: impl AsRef<str>) -> Result<Scenario> {
        let data_dir = data_dir.as_ref();
        let id = id.as_ref();
        let scenario_dir = data_dir.join("scenarios").join(id);

        let metadata_path = scenario_dir.join("scenario.json");
        let metadata: ScenarioMetadata = serde_json::from_slice(
            &datafs::read(&metadata_path).with_context(|| format!("reading {}", metadata_path.display()))?,
        )
        .context("parsing scenario.json")?;

        let mut terrain = Terrain::load(&scenario_dir).context("render terrain")?;
        let vectors = Vectors::load(&scenario_dir).context("osm vectors")?;
        if grade::enabled() {
            terrain.grade_roads(&vectors.roads);
        }
        let population = Population::load(&scenario_dir).context("population")?;
        let world = World {
            width_m: vectors.world_size_m[0],
            height_m: vectors.world_size_m[1],
            fire_rows: vectors.fire_grid.rows,
            fire_cols: vectors.fire_grid.cols,
            cellsize: vectors.fire_grid.cellsize,
        };
        let (fuel, dem) = load_fire_rasters(&scenario_dir, world.fire_rows, world.fire_cols)?;
        let fuel_defs = fuels::load(data_dir).context("fuel table")?;
        let cover = Cover::load(&scenario_dir).context("land cover")?;

        Ok(Scenario { id: id.to_string(), metadata, world, terrain, vectors, population, fuel, dem, fuel_defs, cover })
    }

    pub fn fuel_at(&self, c: Cell) -> i32 {
        self.fuel[c.row * self.world.fire_cols + c.col]
    }

    pub fn is_burnable(&self, c: Cell) -> bool {
        matches!(self.fuel_at(c), 1..=12)
    }
}

/// Fuel and DEM are baked to raw little-endian arrays alongside the GeoTIFFs,
/// because pulling a TIFF decoder in just to read two fixed-size grids is not
/// worth the dependency.
fn load_fire_rasters(dir: &Path, rows: usize, cols: usize) -> Result<(Vec<i32>, Vec<f64>)> {
    let fuel = read_raw::<i32>(&dir.join("fuel.i32"), rows * cols)
        .context("fuel.i32")?;
    let dem = read_raw::<f64>(&dir.join("dem.f64"), rows * cols)
        .context("dem.f64")?;
    Ok((fuel, dem))
}

pub(crate) fn read_raw<T: Copy>(path: &Path, count: usize) -> Result<Vec<T>> {
    let bytes = datafs::read(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let want = count * std::mem::size_of::<T>();
    anyhow::ensure!(
        bytes.len() == want,
        "{}: expected {want} bytes ({count} elements), found {}",
        path.display(),
        bytes.len()
    );
    read_raw_bytes(&bytes, count)
}

pub(crate) fn read_raw_bytes<T: Copy>(bytes: &[u8], count: usize) -> Result<Vec<T>> {
    let want = count * std::mem::size_of::<T>();
    anyhow::ensure!(
        bytes.len() == want,
        "expected {want} bytes ({count} elements), found {}",
        bytes.len()
    );
    let mut out = Vec::<T>::with_capacity(count);
    // SAFETY: the file length matches `count` elements exactly, the source is
    // a byte buffer with no alignment guarantees, and `T` is a plain numeric
    // type with no invalid bit patterns.
    unsafe {
        std::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            out.as_mut_ptr() as *mut u8,
            want,
        );
        out.set_len(count);
    }
    Ok(out)
}
