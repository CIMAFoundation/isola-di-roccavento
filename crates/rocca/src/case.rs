//! What a game is played on: one territory, several fires. Read from the
//! scenario's `game.json`, so nothing about the island is hard-coded here.

use std::path::Path;

use anyhow::{Context, Result};
use fire::Weather;
use scenario::Pos;
use serde::Deserialize;

use abm::suppression::UnitKind;

#[derive(Debug, Clone, Deserialize)]
pub struct Station {
    pub name: String,
    pub pos: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Engine,
    HandCrew,
}

impl Kind {
    pub fn unit_kind(self) -> UnitKind {
        match self {
            Kind::Engine => UnitKind::Engine,
            Kind::HandCrew => UnitKind::HandCrew,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Slot {
    #[serde(default = "engine")]
    pub kind: Kind,
    /// Index into [`Territory::stations`].
    pub station: usize,
    /// People on board (user, 2026-10-14: an engine and three).
    #[serde(default = "crew")]
    pub crew: u8,
}

fn engine() -> Kind {
    Kind::Engine
}

fn crew() -> u8 {
    3
}

/// The tutorial (docs/01-GIOCO.md §1): its fire, where the player is asked
/// to send the teams, and the district to evacuate.
#[derive(Debug, Clone, Deserialize)]
pub struct TutorialSetup {
    pub case: String,
    /// Team index and the point (metres) whose hex it is sent to.
    pub moves: Vec<(usize, [f32; 2])>,
    pub evacuate: String,
}

/// A change of wind during a case, at a fixed simulated minute. Part of the
/// case, not a forecast: the game never shows it before it happens.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Shift {
    pub at_min: i64,
    pub wind_from_deg: f64,
    pub wind_kmh: f64,
}

/// One fire on the territory.
#[derive(Debug, Clone, Deserialize)]
pub struct Case {
    pub name: String,
    /// The locality this start was picked for (the atlas' grouping).
    pub near: String,
    pub ignition: [f32; 2],
    pub radius_m: f32,
    /// Where the wind blows FROM, degrees.
    pub wind_from_deg: f64,
    pub wind_kmh: f64,
    /// Fuel moisture of this fire; the territory's `moisture_pct` if absent.
    #[serde(default)]
    pub moisture_pct: Option<f64>,
    pub minutes: i64,
    #[serde(default)]
    pub shift: Option<Shift>,
    /// Minutes the fire burns, with no one on it, before the game begins:
    /// the time it took to be reported (user, 2026-10-14).
    #[serde(default)]
    pub start_min: i64,
}

impl Case {
    pub fn ignition(&self) -> Pos {
        Pos::from(self.ignition)
    }

    pub fn duration_s(&self) -> i64 {
        self.minutes * 60
    }
}

/// The scenario's `game.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct Territory {
    pub scenario: String,
    pub moisture_pct: f64,
    pub stations: Vec<Station>,
    pub roster: Vec<Slot>,
    /// The cases the kiosk plays, in order; the rest are for the headless
    /// runner and the operator.
    #[serde(default)]
    pub featured: Vec<String>,
    /// Named scenarios offered in the operator menu, in display order.
    #[serde(default)]
    pub levels: Vec<Level>,
    /// The evacuation's limits ([`crate::evac`]).
    #[serde(default)]
    pub evacuation: crate::evac::EvacRules,
    /// The tutorial, played first.
    #[serde(default)]
    pub tutorial: Option<TutorialSetup>,
    /// The fire played after the tutorial.
    pub game: String,
    /// The fires: the tutorial's and the played one (tests and trials play
    /// these too).
    pub cases: Vec<Case>,
}

/// An operator scenario choice: a name («Scenario 1»…), the case it plays and
/// what happens in it.
#[derive(Debug, Clone, Deserialize)]
pub struct Level {
    pub name: String,
    pub case: String,
    pub about: String,
}

impl Territory {
    pub fn load(data_dir: &Path) -> Result<Territory> {
        let p = data_dir.join("scenarios").join(scenario::id()).join("game.json");
        let t: Territory = serde_json::from_slice(&datafs::read(&p).with_context(|| format!("reading {}", p.display()))?)
            .with_context(|| format!("parsing {}", p.display()))?;
        anyhow::ensure!(t.roster.iter().all(|s| s.station < t.stations.len()), "roster names a missing station");
        let playable = |n: &str| t.cases.iter().any(|c| c.name == n);
        anyhow::ensure!(t.featured.iter().all(|n| playable(n)), "featured names a missing case");
        anyhow::ensure!(t.levels.iter().all(|l| playable(&l.case)), "a level names a missing case");
        anyhow::ensure!(playable(&t.game), "game names a missing case");
        anyhow::ensure!(t.tutorial.as_ref().is_none_or(|tu| playable(&tu.case)), "the tutorial names a missing case");
        Ok(t)
    }

    /// A case by name.
    pub fn case(&self, name: &str) -> Option<&Case> {
        self.cases.iter().find(|c| c.name == name)
    }

    /// The kiosk's cases in order: the featured ones, or all of them.
    pub fn playlist(&self) -> Vec<String> {
        if self.featured.is_empty() {
            self.cases.iter().map(|c| c.name.clone()).collect()
        } else {
            self.featured.clone()
        }
    }

    pub fn weather(&self, c: &Case) -> Weather {
        Weather { wind_dir_deg: c.wind_from_deg, wind_speed_kmh: c.wind_kmh, moisture_pct: c.moisture_pct.unwrap_or(self.moisture_pct) }
    }

    /// The weather after the case's wind shift, if it has one.
    pub fn shifted(&self, c: &Case) -> Option<(i64, Weather)> {
        c.shift.map(|s| (s.at_min * 60, Weather { wind_dir_deg: s.wind_from_deg, wind_speed_kmh: s.wind_kmh, moisture_pct: c.moisture_pct.unwrap_or(self.moisture_pct) }))
    }
}
