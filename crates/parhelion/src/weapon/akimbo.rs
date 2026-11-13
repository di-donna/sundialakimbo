//! Akimbo authoring inputs and checked asset transforms.
//!
//! Recipes express intent; compilation checks the resolved weapon and emits paired assets
//! with a native registration profile. Installation requires a matching Dawn capability
//! and publishes the descriptor with its package generation in one transaction.

#[cfg(test)]
mod charge;
pub(crate) mod content;
pub mod deployment;
pub mod geometry;
pub mod shader;
pub(crate) mod visual;

use serde::{Deserialize, Serialize};

pub const INTENT_SCHEMA: u32 = 1;
pub const DEFAULT_GAP_MM: u16 = 360;
pub const MIN_GAP_MM: u16 = 50;
pub const MAX_GAP_MM: u16 = 800;

fn default_gap_mm() -> u16 {
    DEFAULT_GAP_MM
}

fn is_default_gap_mm(value: &u16) -> bool {
    *value == DEFAULT_GAP_MM
}

/// Duplicate the fully resolved weapon, including appearance and gameplay overrides.
/// A recipe carries intent only; compatibility must be derived again for each build.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub schema: u32,
    pub mode: Mode,
    pub reload: Reload,
    /// Distance between the paired weapon origins. Stored as whole millimetres for stable recipes.
    #[serde(default = "default_gap_mm", skip_serializing_if = "is_default_gap_mm")]
    pub gap_mm: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offhand: Option<Offhand>,
}

impl Default for Intent {
    fn default() -> Self {
        Self {
            schema: INTENT_SCHEMA,
            mode: Mode::Independent,
            reload: Reload::LowerMagazine,
            gap_mm: DEFAULT_GAP_MM,
            offhand: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Independent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reload {
    EmptyHands,
    NonFullHands,
    LowerMagazine,
}

impl Reload {
    pub(crate) const fn native_policy(self) -> u32 {
        match self {
            Self::EmptyHands => 1,
            Self::NonFullHands => 2,
            Self::LowerMagazine => 3,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::EmptyHands => "Empty magazines only",
            Self::NonFullHands => "All non-full magazines",
            Self::LowerMagazine => "Lower magazine; both when empty",
        }
    }
}

/// Reserved explicit reference rather than an implicit second equipment slot.
/// Mixed pairs are rejected until separate native ownership is implemented.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Offhand {
    pub namespace: String,
    pub content_sha256: String,
}

impl Intent {
    pub fn gap_meters(&self) -> f32 {
        f32::from(self.gap_mm) / 1000.0
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != INTENT_SCHEMA {
            return Err(format!(
                "Unsupported akimbo schema {}; expected {INTENT_SCHEMA}",
                self.schema
            ));
        }
        if !(MIN_GAP_MM..=MAX_GAP_MM).contains(&self.gap_mm) {
            return Err(format!(
                "Akimbo weapon gap must be between {MIN_GAP_MM} and {MAX_GAP_MM} mm"
            ));
        }
        if self.offhand.is_some() {
            return Err(
                "Akimbo with a separate offhand recipe is not implemented; omit offhand to duplicate the resolved weapon.".into(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn akimbo_intent_is_strict_and_versioned() {
        let intent = Intent::default();
        let encoded = serde_json::to_string(&intent).unwrap();
        assert_eq!(serde_json::from_str::<Intent>(&encoded).unwrap(), intent);
        for field in [
            r#""compatible":true"#,
            r#""runtime_address":123"#,
            r#""experimental":true"#,
        ] {
            let document =
                format!(r#"{{"schema":1,"mode":"independent","reload":"empty_hands",{field}}}"#);
            assert!(serde_json::from_str::<Intent>(&document).is_err());
        }
        for unsupported in [
            r#"{"schema":1,"mode":"burst","reload":"empty_hands"}"#,
            r#"{"schema":1,"mode":"independent","reload":"both"}"#,
            r#"{"mode":"independent","reload":"empty_hands"}"#,
        ] {
            assert!(serde_json::from_str::<Intent>(unsupported).is_err());
        }
        let mut future = intent;
        future.schema += 1;
        assert!(future.validate().unwrap_err().contains("schema"));
    }

    #[test]
    fn akimbo_mixed_pair_never_silently_uses_the_main_donor() {
        let intent = Intent {
            offhand: Some(Offhand {
                namespace: "parhelion.offhand".into(),
                content_sha256: "a".repeat(64),
            }),
            ..Default::default()
        };
        assert!(intent.validate().unwrap_err().contains("separate offhand"));
    }

    #[test]
    fn akimbo_gap_defaults_preserve_old_recipes_and_custom_values_round_trip() {
        let old = r#"{"schema":1,"mode":"independent","reload":"empty_hands"}"#;
        let mut intent: Intent = serde_json::from_str(old).unwrap();
        assert_eq!(intent.gap_mm, DEFAULT_GAP_MM);
        assert_eq!(serde_json::to_string(&intent).unwrap(), old);
        for gap in [MIN_GAP_MM, 240, MAX_GAP_MM] {
            intent.gap_mm = gap;
            intent.validate().unwrap();
            let encoded = serde_json::to_string(&intent).unwrap();
            assert_eq!(serde_json::from_str::<Intent>(&encoded).unwrap(), intent);
            assert!((intent.gap_meters() - f32::from(gap) / 1000.0).abs() < 1e-6);
        }
        for gap in [0, MIN_GAP_MM - 1, MAX_GAP_MM + 1, u16::MAX] {
            intent.gap_mm = gap;
            assert!(intent.validate().is_err());
        }
        for value in ["-1", "1.5", "65536", "null", "\"360\""] {
            let document = format!(
                r#"{{"schema":1,"mode":"independent","reload":"empty_hands","gap_mm":{value}}}"#
            );
            assert!(serde_json::from_str::<Intent>(&document).is_err());
        }
    }

    #[test]
    fn akimbo_reload_default_changes_only_new_recipes_and_has_explicit_native_policy() {
        let new = Intent::default();
        assert_eq!(new.reload, Reload::LowerMagazine);
        assert_eq!(new.reload.native_policy(), 3);
        let old: Intent =
            serde_json::from_str(r#"{"schema":1,"mode":"independent","reload":"empty_hands"}"#)
                .unwrap();
        assert_eq!(old.reload, Reload::EmptyHands);
        assert_eq!(old.reload.native_policy(), 1);
        let non_full: Intent =
            serde_json::from_str(r#"{"schema":1,"mode":"independent","reload":"non_full_hands"}"#)
                .unwrap();
        assert_eq!(non_full.reload, Reload::NonFullHands);
        assert_eq!(non_full.reload.native_policy(), 2);
        for intent in [old, non_full, new] {
            let encoded = serde_json::to_vec(&intent).unwrap();
            assert_eq!(serde_json::from_slice::<Intent>(&encoded).unwrap(), intent);
        }
    }
}
