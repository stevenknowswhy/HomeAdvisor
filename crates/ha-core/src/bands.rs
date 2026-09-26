//! Banded profile types: the generalized household facts the advisory
//! engine is allowed to see.
//!
//! Raw ages, incomes, and addresses never reach a pack rule. Evaluation
//! matches against the same band vocabulary the store seeds (`band`,
//! migration 002) and the `household` CHECK admits — these types are the
//! contract between the store's band foreign keys and the packs' rule
//! conditions. Adding a band is a reviewed store migration plus one enum
//! variant; the two cannot drift silently because pack data references
//! bands through these enums and nowhere else.

use serde::{Deserialize, Serialize};

/// A person's age band: the store's seeded age-band ids (`band.id`,
/// band_type `'age'`), ordered youngest to oldest.
///
/// Variant order is the seed's `sort_order`, so `Ord` on this type is band
/// order, not alphabetical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AgeBand {
    #[serde(rename = "age_0_2")]
    ZeroTo2,
    #[serde(rename = "age_3_5")]
    ThreeTo5,
    #[serde(rename = "age_6_9")]
    SixTo9,
    #[serde(rename = "age_10_12")]
    TenTo12,
    #[serde(rename = "age_13_15")]
    ThirteenTo15,
    #[serde(rename = "age_16_17")]
    SixteenTo17,
    #[serde(rename = "age_18_24")]
    EighteenTo24,
    #[serde(rename = "age_25_34")]
    TwentyFiveTo34,
    #[serde(rename = "age_35_44")]
    ThirtyFiveTo44,
    #[serde(rename = "age_45_54")]
    FortyFiveTo54,
    #[serde(rename = "age_55_64")]
    FiftyFiveTo64,
    #[serde(rename = "age_65_74")]
    SixtyFiveTo74,
    #[serde(rename = "age_75_plus")]
    SeventyFivePlus,
}

impl AgeBand {
    /// The exact band id the store seeds (`band.id`, band_type `'age'`).
    pub fn as_str(self) -> &'static str {
        match self {
            AgeBand::ZeroTo2 => "age_0_2",
            AgeBand::ThreeTo5 => "age_3_5",
            AgeBand::SixTo9 => "age_6_9",
            AgeBand::TenTo12 => "age_10_12",
            AgeBand::ThirteenTo15 => "age_13_15",
            AgeBand::SixteenTo17 => "age_16_17",
            AgeBand::EighteenTo24 => "age_18_24",
            AgeBand::TwentyFiveTo34 => "age_25_34",
            AgeBand::ThirtyFiveTo44 => "age_35_44",
            AgeBand::FortyFiveTo54 => "age_45_54",
            AgeBand::FiftyFiveTo64 => "age_55_64",
            AgeBand::SixtyFiveTo74 => "age_65_74",
            AgeBand::SeventyFivePlus => "age_75_plus",
        }
    }
}

/// A household's income band: the store's seeded income-band ids
/// (`band.id`, band_type `'income'`), ordered lowest to highest. Variant
/// order is the seed's `sort_order`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum IncomeBand {
    #[serde(rename = "income_under_50k")]
    Under50K,
    #[serde(rename = "income_50k_75k")]
    From50To75K,
    #[serde(rename = "income_75k_100k")]
    From75To100K,
    #[serde(rename = "income_100k_150k")]
    From100To150K,
    #[serde(rename = "income_150k_200k")]
    From150To200K,
    #[serde(rename = "income_200k_plus")]
    Over200K,
}

impl IncomeBand {
    /// The exact band id the store seeds (`band.id`, band_type `'income'`).
    pub fn as_str(self) -> &'static str {
        match self {
            IncomeBand::Under50K => "income_under_50k",
            IncomeBand::From50To75K => "income_50k_75k",
            IncomeBand::From75To100K => "income_75k_100k",
            IncomeBand::From100To150K => "income_100k_150k",
            IncomeBand::From150To200K => "income_150k_200k",
            IncomeBand::Over200K => "income_200k_plus",
        }
    }
}

/// The generalized region classes, exactly the values the `household`
/// table's CHECK constraint admits (`urban_metro`, `suburban`, `rural`,
/// `small_town`, `unclassified`).
///
/// Lives in `ha-core` so the privacy redactor and the advisory engine
/// share one region vocabulary; `ha-privacy` re-exports it and keeps the
/// `ha_privacy::RegionClass` path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionClass {
    UrbanMetro,
    Suburban,
    Rural,
    SmallTown,
    Unclassified,
}

impl RegionClass {
    pub fn as_str(self) -> &'static str {
        match self {
            RegionClass::UrbanMetro => "urban_metro",
            RegionClass::Suburban => "suburban",
            RegionClass::Rural => "rural",
            RegionClass::SmallTown => "small_town",
            RegionClass::Unclassified => "unclassified",
        }
    }
}

/// A household member's role (the `person` table's `role` CHECK).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberRole {
    Adult,
    Child,
}

impl MemberRole {
    pub fn as_str(self) -> &'static str {
        match self {
            MemberRole::Adult => "adult",
            MemberRole::Child => "child",
        }
    }

    fn is_child(self) -> bool {
        self == MemberRole::Child
    }
}

/// A child's school stage (the `person` table's `school_stage` CHECK).
/// Valid only for child members — the schema CHECKs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchoolStage {
    Preschool,
    Elementary,
    MiddleSchool,
    HighSchool,
}

impl SchoolStage {
    pub fn as_str(self) -> &'static str {
        match self {
            SchoolStage::Preschool => "preschool",
            SchoolStage::Elementary => "elementary",
            SchoolStage::MiddleSchool => "middle_school",
            SchoolStage::HighSchool => "high_school",
        }
    }
}

/// One member of the banded profile: a role and a band, never a raw age.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BandedMember {
    pub role: MemberRole,
    pub age_band: AgeBand,
    /// Set for children whose school stage the family recorded.
    pub school_stage: Option<SchoolStage>,
}

/// A household profile reduced to bands: what the advisory engine may see.
///
/// No raw age, income, or address exists on this type by construction —
/// every field is a band vocabulary or a role. The engine's whole view of
/// a family is one of these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BandedProfile {
    pub region: RegionClass,
    pub income_band: IncomeBand,
    pub members: Vec<BandedMember>,
}

impl BandedProfile {
    /// The child members, in profile order.
    pub fn children(&self) -> impl Iterator<Item = &BandedMember> {
        self.members.iter().filter(|m| m.role.is_child())
    }

    /// The adult members, in profile order.
    pub fn adults(&self) -> impl Iterator<Item = &BandedMember> {
        self.members.iter().filter(|m| !m.role.is_child())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_band_as_str_pins_the_seeded_band_ids() {
        // Mirrors `M002_SEED_BANDS` (ha-store migrations.rs). The pack
        // schema matches these ids; AC2's writer tests re-pin the full
        // seed against a live database.
        assert_eq!(AgeBand::ZeroTo2.as_str(), "age_0_2");
        assert_eq!(AgeBand::ThreeTo5.as_str(), "age_3_5");
        assert_eq!(AgeBand::SixTo9.as_str(), "age_6_9");
        assert_eq!(AgeBand::TenTo12.as_str(), "age_10_12");
        assert_eq!(AgeBand::ThirteenTo15.as_str(), "age_13_15");
        assert_eq!(AgeBand::SixteenTo17.as_str(), "age_16_17");
        assert_eq!(AgeBand::EighteenTo24.as_str(), "age_18_24");
        assert_eq!(AgeBand::SeventyFivePlus.as_str(), "age_75_plus");
    }

    #[test]
    fn income_band_as_str_pins_the_seeded_band_ids() {
        assert_eq!(IncomeBand::Under50K.as_str(), "income_under_50k");
        assert_eq!(IncomeBand::From50To75K.as_str(), "income_50k_75k");
        assert_eq!(IncomeBand::From75To100K.as_str(), "income_75k_100k");
        assert_eq!(IncomeBand::From100To150K.as_str(), "income_100k_150k");
        assert_eq!(IncomeBand::From150To200K.as_str(), "income_150k_200k");
        assert_eq!(IncomeBand::Over200K.as_str(), "income_200k_plus");
    }

    #[test]
    fn region_class_as_str_matches_the_household_check() {
        assert_eq!(RegionClass::UrbanMetro.as_str(), "urban_metro");
        assert_eq!(RegionClass::Suburban.as_str(), "suburban");
        assert_eq!(RegionClass::Rural.as_str(), "rural");
        assert_eq!(RegionClass::SmallTown.as_str(), "small_town");
        assert_eq!(RegionClass::Unclassified.as_str(), "unclassified");
    }

    #[test]
    fn bands_roundtrip_through_serde_as_seeded_ids() {
        for band in [
            AgeBand::ZeroTo2,
            AgeBand::ThirteenTo15,
            AgeBand::SeventyFivePlus,
        ] {
            let json = serde_json::to_string(&band).expect("serialize");
            assert!(json.contains(band.as_str()));
            let back: AgeBand = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, band);
        }
        for band in [IncomeBand::Under50K, IncomeBand::Over200K] {
            let json = serde_json::to_string(&band).expect("serialize");
            let back: IncomeBand = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, band);
        }
    }

    #[test]
    fn age_band_order_is_the_seed_sort_order() {
        let mut bands = [
            AgeBand::SixteenTo17,
            AgeBand::ZeroTo2,
            AgeBand::SeventyFivePlus,
        ];
        bands.sort();
        assert_eq!(
            bands,
            [
                AgeBand::ZeroTo2,
                AgeBand::SixteenTo17,
                AgeBand::SeventyFivePlus
            ]
        );
    }

    #[test]
    fn banded_profile_splits_children_and_adults() {
        let profile = BandedProfile {
            region: RegionClass::Suburban,
            income_band: IncomeBand::From100To150K,
            members: vec![
                BandedMember {
                    role: MemberRole::Adult,
                    age_band: AgeBand::ThirtyFiveTo44,
                    school_stage: None,
                },
                BandedMember {
                    role: MemberRole::Child,
                    age_band: AgeBand::TenTo12,
                    school_stage: Some(SchoolStage::Elementary),
                },
            ],
        };
        assert_eq!(profile.children().count(), 1);
        assert_eq!(profile.adults().count(), 1);
        assert_eq!(
            profile.children().next().expect("child").age_band,
            AgeBand::TenTo12
        );
    }

    #[test]
    fn wire_enum_strings_match_their_serde_forms() {
        // as_str and serde must speak the same vocabulary, or pack data
        // and store rows drift apart silently.
        for (role, expected) in [(MemberRole::Adult, "adult"), (MemberRole::Child, "child")] {
            assert_eq!(role.as_str(), expected);
            let json = serde_json::to_string(&role).expect("serialize");
            assert_eq!(json, format!("\"{expected}\""));
        }
        for stage in [SchoolStage::MiddleSchool, SchoolStage::HighSchool] {
            let json = serde_json::to_string(&stage).expect("serialize");
            assert_eq!(json, format!("\"{}\"", stage.as_str()));
        }
    }
}
