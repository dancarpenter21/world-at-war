//! Versioned scenario definitions and spawning.

use serde::{Deserialize, Serialize};
use sim_core::{
    CommunicationNode, Domain, GeoPose, PlatformSpawn, Sensor, Side, Simulation, Velocity,
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub id: String,
    pub title: String,
    pub description: String,
    pub version: u32,
    pub requires_space_catalog: bool,
    pub units: Vec<ScenarioUnit>,
    pub roles: Vec<ScenarioRole>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioUnit {
    pub id: Uuid,
    pub name: String,
    pub side: Side,
    pub domain: Domain,
    pub sidc: String,
    pub position: GeoPose,
    pub velocity: Velocity,
    pub sensor: Option<Sensor>,
    pub communications: Option<CommunicationNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioRole {
    pub id: Uuid,
    pub name: String,
    pub side: Side,
    pub command_scope: Vec<Uuid>,
    pub authority_echelon: u8,
    pub ai_controlled: bool,
}

#[derive(Debug, Error)]
pub enum ScenarioError {
    #[error("scenario must include a Blue unit")]
    MissingBlue,
    #[error("scenario must include a Red unit")]
    MissingRed,
    #[error("scenario includes duplicate unit id {0}")]
    DuplicateUnit(Uuid),
    #[error("entity {0} has an invalid MIL-STD-2525D SIDC")]
    InvalidSidc(Uuid),
    #[error("role {0} has no valid command scope")]
    InvalidRole(Uuid),
}

impl Scenario {
    pub fn validate(&self) -> Result<(), ScenarioError> {
        let mut ids = std::collections::BTreeSet::new();
        let mut blue = false;
        let mut red = false;
        for unit in &self.units {
            if !ids.insert(unit.id) {
                return Err(ScenarioError::DuplicateUnit(unit.id));
            }
            if unit.sidc.len() != 30
                || !unit
                    .sidc
                    .chars()
                    .all(|character| character.is_ascii_digit())
            {
                return Err(ScenarioError::InvalidSidc(unit.id));
            }
            blue |= unit.side == Side::Blue;
            red |= unit.side == Side::Red;
        }
        if !blue {
            return Err(ScenarioError::MissingBlue);
        }
        if !red {
            return Err(ScenarioError::MissingRed);
        }
        for role in &self.roles {
            if role.command_scope.is_empty()
                || role.command_scope.iter().any(|id| !ids.contains(id))
            {
                return Err(ScenarioError::InvalidRole(role.id));
            }
        }
        Ok(())
    }

    pub fn spawn(&self) -> Result<Simulation, ScenarioError> {
        self.validate()?;
        let mut simulation = Simulation::new();
        for unit in &self.units {
            simulation.spawn_platform(PlatformSpawn {
                id: unit.id,
                name: unit.name.clone(),
                side: unit.side,
                domain: unit.domain,
                pose: unit.position,
                velocity: unit.velocity,
                sensor: unit.sensor,
                communication: unit.communications,
                sidc: unit.sidc.clone(),
            });
        }
        Ok(simulation)
    }
}

pub fn global_crisis_scenario() -> Scenario {
    let blue = [
        (
            "White House Situation Room",
            Domain::Land,
            38.8977,
            -77.0365,
        ),
        ("Pentagon NMCC", Domain::Land, 38.8719, -77.0563),
        ("Joint Base Andrews", Domain::Land, 38.8108, -76.8670),
        ("Naval Station Norfolk", Domain::Sea, 36.9460, -76.3290),
        ("Andersen Air Force Base", Domain::Land, 13.5840, 144.9300),
        ("Joint Base Pearl Harbor", Domain::Sea, 21.3490, -157.9440),
        ("Ramstein Air Base", Domain::Land, 49.4369, 7.6003),
        ("Yokota Air Base", Domain::Land, 35.7485, 139.3480),
        (
            "Diego Garcia Support Facility",
            Domain::Sea,
            -7.3195,
            72.4229,
        ),
        (
            "Cheyenne Mountain Station",
            Domain::Land,
            38.7445,
            -104.8460,
        ),
        ("F-35A CAP Alpha 1", Domain::Air, 38.1, -76.7),
        ("F-35A CAP Alpha 2", Domain::Air, 38.2, -76.6),
        ("F-35C Pacific 1", Domain::Air, 22.0, 145.0),
        ("F-35C Pacific 2", Domain::Air, 22.2, 145.2),
        ("F-15EX Europe 1", Domain::Air, 50.0, 8.0),
        ("F-15EX Europe 2", Domain::Air, 50.1, 8.2),
        ("B-2A Spirit 1", Domain::Air, 39.0, -95.0),
        ("B-52H Global 1", Domain::Air, 34.0, -105.0),
        ("E-3G Sentry Atlantic", Domain::Air, 42.0, -45.0),
        ("E-2D Hawkeye Pacific", Domain::Air, 20.0, 150.0),
        ("KC-46A Atlantic 1", Domain::Air, 40.0, -55.0),
        ("KC-46A Pacific 1", Domain::Air, 25.0, 155.0),
        ("P-8A Poseidon Atlantic", Domain::Air, 35.0, -40.0),
        ("P-8A Poseidon Pacific", Domain::Air, 18.0, 148.0),
        ("MQ-9A Europe", Domain::Air, 47.0, 15.0),
        ("RQ-4 Global Hawk", Domain::Air, 30.0, 135.0),
        ("USS Gerald R. Ford", Domain::Sea, 37.0, -50.0),
        ("USS Arleigh Burke", Domain::Sea, 36.5, -49.0),
        ("USS Thomas Hudner", Domain::Sea, 37.5, -51.0),
        ("USS Ronald Reagan", Domain::Sea, 20.0, 145.0),
        ("USS John Finn", Domain::Sea, 19.5, 144.5),
        ("USS Rafael Peralta", Domain::Sea, 20.5, 145.5),
        ("USS Virginia", Domain::Undersea, 38.0, -42.0),
        ("USS Hawaii", Domain::Undersea, 18.0, 150.0),
        ("USNS John Lewis", Domain::Sea, 21.0, 146.0),
        ("USNS Supply", Domain::Sea, 36.0, -52.0),
        ("1st Armored Brigade", Domain::Land, 52.0, 20.0),
        ("2nd Stryker Brigade", Domain::Land, 51.0, 22.0),
        ("Marine Littoral Regiment", Domain::Land, 14.0, 145.0),
        ("HIMARS Battalion Pacific", Domain::Land, 13.6, 144.9),
        ("Patriot Battalion Europe", Domain::Land, 50.0, 9.0),
        ("THAAD Battery Guam", Domain::Land, 13.5, 144.8),
        ("JTAC Team Saber", Domain::Land, 51.5, 21.0),
        ("Joint Logistics Group", Domain::Land, 49.5, 8.0),
        ("Cyber Mission Force", Domain::Cyber, 39.0, -77.0),
        ("Defensive Network Cell", Domain::Cyber, 38.9, -77.05),
        ("SATCOM Control Element", Domain::Space, 38.7, -104.8),
        ("Space Domain Awareness Cell", Domain::Space, 38.8, -104.9),
    ];
    let red = [
        ("Red National Command", Domain::Land, 55.75, 37.62),
        ("Red Air Operations Center", Domain::Land, 54.5, 38.0),
        ("Red Fighter Flight 1", Domain::Air, 54.0, 35.0),
        ("Red Fighter Flight 2", Domain::Air, 53.5, 36.0),
        ("Red Bomber Flight", Domain::Air, 60.0, 45.0),
        ("Red AEW Aircraft", Domain::Air, 56.0, 40.0),
        ("Red Tanker", Domain::Air, 57.0, 42.0),
        ("Red Surface Group Flag", Domain::Sea, 64.0, 5.0),
        ("Red Surface Escort 1", Domain::Sea, 63.5, 4.5),
        ("Red Surface Escort 2", Domain::Sea, 64.5, 5.5),
        ("Red Attack Submarine", Domain::Undersea, 60.0, -5.0),
        ("Red Armor Brigade", Domain::Land, 53.0, 28.0),
        ("Red Missile Brigade", Domain::Land, 54.0, 30.0),
        ("Red Air Defense Regiment", Domain::Land, 55.0, 32.0),
        ("Red EW Battalion", Domain::Cyber, 55.5, 34.0),
        ("Red Cyber Unit", Domain::Cyber, 55.7, 37.0),
    ];
    let mut units = Vec::with_capacity(64);
    for (index, (name, domain, latitude, longitude)) in blue.into_iter().enumerate() {
        units.push(unit(
            index as u128 + 1,
            name,
            Side::Blue,
            domain,
            latitude,
            longitude,
        ));
    }
    for (index, (name, domain, latitude, longitude)) in red.into_iter().enumerate() {
        units.push(unit(
            index as u128 + 49,
            name,
            Side::Red,
            domain,
            latitude,
            longitude,
        ));
    }
    let role_names = [
        (101, "National Command", 1, 5),
        (102, "NMCC / Secretary of Defense", 2, 4),
        (103, "Air Operations", 3, 3),
        (104, "Maritime Operations", 4, 3),
        (105, "Land Operations", 37, 2),
        (106, "Space Operations", 47, 2),
        (107, "Tactical Flight Lead", 11, 1),
    ];
    let mut roles: Vec<ScenarioRole> = role_names
        .into_iter()
        .map(|(id, name, unit_id, echelon)| ScenarioRole {
            id: Uuid::from_u128(id),
            name: name.into(),
            side: Side::Blue,
            command_scope: vec![Uuid::from_u128(unit_id)],
            authority_echelon: echelon,
            ai_controlled: false,
        })
        .collect();
    roles.push(ScenarioRole {
        id: Uuid::from_u128(108),
        name: "Red Commander AI".into(),
        side: Side::Red,
        command_scope: vec![Uuid::from_u128(49)],
        authority_echelon: 4,
        ai_controlled: true,
    });
    Scenario {
        id: "global-crisis.v1".into(),
        title: "Global Crisis".into(),
        description: "A combined-domain global crisis directed from the White House and Pentagon."
            .into(),
        version: 1,
        requires_space_catalog: true,
        units,
        roles,
    }
}

fn unit(
    id: u128,
    name: &str,
    side: Side,
    domain: Domain,
    latitude: f64,
    longitude: f64,
) -> ScenarioUnit {
    let airborne = domain == Domain::Air;
    ScenarioUnit {
        id: Uuid::from_u128(id),
        name: name.into(),
        side,
        domain,
        sidc: sidc(side, domain).into(),
        position: GeoPose {
            latitude_deg: latitude,
            longitude_deg: longitude,
            altitude_m: if airborne { 8_000.0 } else { 0.0 },
        },
        velocity: Velocity {
            north_mps: 0.0,
            east_mps: if airborne { 180.0 } else { 0.0 },
            climb_mps: 0.0,
        },
        sensor: Some(Sensor {
            range_m: if airborne { 180_000.0 } else { 80_000.0 },
            identification_range_m: 35_000.0,
        }),
        communications: Some(CommunicationNode {
            range_m: 500_000.0,
            operational: true,
        }),
    }
}

fn sidc(side: Side, domain: Domain) -> &'static str {
    match (side, domain) {
        (Side::Blue, Domain::Air) => "100301000011010000000000000000",
        (Side::Red, Domain::Air) => "100601000011010000000000000000",
        (Side::Blue, Domain::Sea) => "100330000012010000000000000000",
        (Side::Red, Domain::Sea) => "100630000012010000000000000000",
        (Side::Blue, Domain::Undersea) => "100335000012010000000000000000",
        (Side::Red, Domain::Undersea) => "100635000012010000000000000000",
        (Side::Blue, Domain::Space) => "100305000011010000000000000000",
        (Side::Red, Domain::Space) => "100605000011010000000000000000",
        (Side::Blue, Domain::Cyber) => "100340000012110000000000000000",
        (Side::Red, Domain::Cyber) => "100640000012110000000000000000",
        (Side::Blue, Domain::Land) => "100310000012110000000000000000",
        (Side::Red, Domain::Land) => "100610000012110000000000000000",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_crisis_has_required_entities_and_roles() {
        let scenario = global_crisis_scenario();
        scenario.validate().unwrap();
        assert_eq!(scenario.units.len(), 64);
        assert_eq!(scenario.roles.len(), 8);
        assert!(scenario
            .units
            .iter()
            .any(|unit| unit.name.contains("White House")));
        assert!(scenario
            .units
            .iter()
            .any(|unit| unit.name.contains("Pentagon")));
    }
}
