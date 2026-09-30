//! Local heuristic category suggestions. Not audio analysis; scores are heuristic
//! evidence, not calibrated probabilities. All rules and weights live here.

use crate::protocol::payload::{DecodedParams, Payload};
use serde::{Deserialize, Serialize};

pub const CLASSIFIER_VERSION: u32 = 1;
pub const MIN_SCORE: f64 = 0.55;
pub const MIN_MARGIN: f64 = 0.15;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum Category {
    Bass,
    Lead,
    Pad,
    Keys,
    Pluck,
    #[serde(rename = "Arp / Sequence")]
    ArpSequence,
    #[serde(rename = "FX / Texture")]
    FxTexture,
    Other,
}

impl Category {
    pub const ALL: [Category; 8] = [
        Category::Bass,
        Category::Lead,
        Category::Pad,
        Category::Keys,
        Category::Pluck,
        Category::ArpSequence,
        Category::FxTexture,
        Category::Other,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Category::Bass => "Bass",
            Category::Lead => "Lead",
            Category::Pad => "Pad",
            Category::Keys => "Keys",
            Category::Pluck => "Pluck",
            Category::ArpSequence => "Arp / Sequence",
            Category::FxTexture => "FX / Texture",
            Category::Other => "Other",
        }
    }
    pub fn from_label(s: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|c| c.label().eq_ignore_ascii_case(s))
    }
    /// Sort order used when grouping by category.
    pub fn order(self) -> usize {
        Self::ALL.iter().position(|c| *c == self).unwrap()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Classification {
    pub category: Category,
    pub score: f64,
    pub reasons: Vec<String>,
    pub classifier_version: u32,
    pub parameters_available: bool,
    /// Structural badges independent of the category (e.g. "Arp", "Seq").
    pub badges: Vec<String>,
}

const NAME_TOKENS: &[(&str, Category, f64)] = &[
    ("bass", Category::Bass, 0.6),
    ("bs", Category::Bass, 0.45),
    ("sub", Category::Bass, 0.3),
    ("lead", Category::Lead, 0.6),
    ("ld", Category::Lead, 0.45),
    ("solo", Category::Lead, 0.4),
    ("pad", Category::Pad, 0.6),
    ("pd", Category::Pad, 0.45),
    ("strings", Category::Pad, 0.45),
    ("str", Category::Pad, 0.35),
    ("choir", Category::Pad, 0.4),
    ("keys", Category::Keys, 0.6),
    ("key", Category::Keys, 0.5),
    ("piano", Category::Keys, 0.55),
    ("ep", Category::Keys, 0.4),
    ("organ", Category::Keys, 0.5),
    ("clav", Category::Keys, 0.5),
    ("brass", Category::Keys, 0.3),
    ("pluck", Category::Pluck, 0.6),
    ("plk", Category::Pluck, 0.45),
    ("harp", Category::Pluck, 0.35),
    ("arp", Category::ArpSequence, 0.55),
    ("seq", Category::ArpSequence, 0.55),
    ("texture", Category::FxTexture, 0.55),
    ("atmos", Category::FxTexture, 0.5),
    ("fx", Category::FxTexture, 0.55),
    ("sfx", Category::FxTexture, 0.55),
    ("noise", Category::FxTexture, 0.4),
    ("drone", Category::FxTexture, 0.45),
];

/// Tokenize on non-alphanumerics and on lowercase->uppercase / letter<->digit boundaries.
pub fn tokens(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev: Option<char> = None;
    for c in name.chars() {
        if !c.is_ascii_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev = None;
            continue;
        }
        if let Some(p) = prev {
            let boundary = (p.is_ascii_lowercase() && c.is_ascii_uppercase())
                || (p.is_ascii_digit() != c.is_ascii_digit());
            if boundary && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        }
        cur.push(c.to_ascii_lowercase());
        prev = Some(c);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn token_matches(tok: &str, key: &str) -> bool {
    // Short aliases must be the whole token (a plural 's' is allowed for 3+ chars); keys of 4+
    // chars may prefix a slightly longer token ("basses"). "old" never matches "ld".
    if tok == key
        || (key.len() >= 3
            && tok.len() == key.len() + 1
            && tok.ends_with('s')
            && tok.starts_with(key))
    {
        return true;
    }
    key.len() >= 4 && tok.starts_with(key) && tok.len() <= key.len() + 2
}

pub fn classify(payload: &Payload) -> Classification {
    let name = payload.display_name().unwrap_or_default();
    let params = payload.decode();
    classify_parts(&name, params.as_ref())
}

pub fn classify_parts(name: &str, params: Option<&DecodedParams>) -> Classification {
    let mut scores = [0.0f64; 8];
    let mut reasons: Vec<String> = Vec::new();
    let mut badges = Vec::new();
    let add =
        |scores: &mut [f64; 8], reasons: &mut Vec<String>, c: Category, w: f64, why: String| {
            scores[c.order()] += w;
            reasons.push(format!("{}: +{:.2} {}", c.label(), w, why));
        };

    let toks = tokens(name);
    for t in &toks {
        for (key, cat, w) in NAME_TOKENS {
            if token_matches(t, key) {
                add(
                    &mut scores,
                    &mut reasons,
                    *cat,
                    *w,
                    format!("name token '{t}'"),
                );
            }
        }
    }

    if let Some(p) = params {
        let arp = p.flag("ARP_ON");
        let seq = p.flag("SEQ_ON");
        if arp {
            badges.push("Arp".to_string());
        }
        if seq {
            badges.push("Seq".to_string());
        }
        if arp || seq {
            add(
                &mut scores,
                &mut reasons,
                Category::ArpSequence,
                0.6,
                "arpeggiator/sequencer enabled".into(),
            );
        }
        let a_att = p.norm("AMP_ATTACK");
        let a_dec = p.norm("AMP_DECAY");
        let a_sus = p.norm("AMP_SUSTAIN");
        let a_rel = p.norm("AMP_RELEASE");
        let cutoff = p.norm("LP_CUTOFF");
        let reso = p.norm("LP_RESONANCE");
        let sub = p.norm("SUB_LEVEL");
        let noise = p.norm("NOISE_LEVEL");
        let unison = p.flag("UNISON_ON");
        let glide = p.flag("GLIDE_ON");
        let lfo = p.norm("LFO_AMOUNT");
        let fx = (p.flag("FX1_ON") as u8 as f64) * p.norm("FX1_MIX")
            + (p.flag("FX2_ON") as u8 as f64) * p.norm("FX2_MIX");

        if a_att > 0.35 && a_rel > 0.35 && a_sus > 0.4 {
            add(
                &mut scores,
                &mut reasons,
                Category::Pad,
                0.45,
                "slow attack, long release, sustained".into(),
            );
        }
        if fx > 0.6 && a_rel > 0.3 {
            add(
                &mut scores,
                &mut reasons,
                Category::Pad,
                0.1,
                "heavy effects with release".into(),
            );
        }
        if a_att < 0.08 && a_sus < 0.2 && a_dec < 0.5 && a_rel < 0.4 {
            add(
                &mut scores,
                &mut reasons,
                Category::Pluck,
                0.45,
                "fast attack, decaying, low sustain".into(),
            );
        }
        if unison {
            add(
                &mut scores,
                &mut reasons,
                Category::Lead,
                0.2,
                "unison on".into(),
            );
        }
        if glide {
            add(
                &mut scores,
                &mut reasons,
                Category::Lead,
                0.15,
                "glide on".into(),
            );
        }
        if a_att < 0.1 && a_sus > 0.5 && (unison || glide) {
            add(
                &mut scores,
                &mut reasons,
                Category::Lead,
                0.15,
                "quick attack, sustained, mono-style".into(),
            );
        }
        if sub > 0.4 {
            add(
                &mut scores,
                &mut reasons,
                Category::Bass,
                0.25,
                "sub oscillator".into(),
            );
        }
        if cutoff < 0.45 && a_att < 0.1 && a_rel < 0.35 {
            add(
                &mut scores,
                &mut reasons,
                Category::Bass,
                0.2,
                "dark filter, fast envelope".into(),
            );
        }
        if !unison && a_att < 0.1 && a_sus > 0.15 && a_rel < 0.5 && lfo < 0.3 && cutoff > 0.4 {
            add(
                &mut scores,
                &mut reasons,
                Category::Keys,
                0.3,
                "playable polyphonic envelope".into(),
            );
        }
        if noise > 0.5 || (reso > 0.7) || (lfo > 0.7 && a_sus > 0.5) {
            add(
                &mut scores,
                &mut reasons,
                Category::FxTexture,
                0.35,
                "strong noise/resonance/modulation".into(),
            );
        }
    } else {
        reasons.push("parameters unavailable: unsupported layout; name-only".into());
    }

    // Winner / margin
    let mut ranked: Vec<(Category, f64)> = Category::ALL
        .iter()
        .map(|c| (*c, scores[c.order()]))
        .collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap()
            .then(a.0.order().cmp(&b.0.order()))
    });
    let (best, s1) = ranked[0];
    let s2 = ranked[1].1;
    let score = (s1.min(1.0) * 1000.0).round() / 1000.0;
    let category = if s1 < MIN_SCORE {
        reasons.push(format!("top score {s1:.2} below {MIN_SCORE}"));
        Category::Other
    } else if s1 - s2 < MIN_MARGIN {
        reasons.push(format!(
            "ambiguous: {} {s1:.2} vs {} {s2:.2}",
            best.label(),
            ranked[1].0.label()
        ));
        Category::Other
    } else {
        best
    };
    Classification {
        category,
        score,
        reasons,
        classifier_version: CLASSIFIER_VERSION,
        parameters_available: params.is_some(),
        badges,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::payload::{off, synthetic_payload};
    use crate::protocol::PAYLOAD_LEN;

    #[test]
    fn tokenization_boundaries() {
        assert_eq!(tokens("BS-Wobble"), vec!["bs", "wobble"]);
        assert_eq!(tokens("FatLD2"), vec!["fat", "ld", "2"]);
        let c = classify_parts("Old Times", None);
        assert!(c.reasons.iter().all(|r| !r.starts_with("Lead")));
        assert_eq!(classify_parts("Big Bass", None).category, Category::Bass);
        assert_eq!(classify_parts("Warm Pads", None).category, Category::Pad);
    }

    #[test]
    fn deterministic_and_arp() {
        let mut b = [0u8; PAYLOAD_LEN];
        b[off::ARP_ON] = 1;
        b[107..117].copy_from_slice(b"Arp Pulser");
        let p = Payload::from_slice(&b).unwrap();
        let c1 = classify(&p);
        let c2 = classify(&p);
        assert_eq!(c1.category, Category::ArpSequence);
        assert_eq!(c1.score, c2.score);
        assert_eq!(c1.badges, vec!["Arp"]);
    }

    #[test]
    fn ambiguous_is_other() {
        let c = classify_parts("Bass Lead", None);
        assert_eq!(c.category, Category::Other);
    }

    #[test]
    fn unsupported_features_flagged() {
        let mut b = *synthetic_payload(1, "Pad").bytes();
        b[108] = 0xFF;
        let c = classify(&Payload::from_slice(&b).unwrap());
        assert!(!c.parameters_available);
    }

    #[test]
    fn category_labels_roundtrip() {
        for c in Category::ALL {
            assert_eq!(Category::from_label(c.label()), Some(c));
            assert_eq!(
                serde_json::to_string(&c).unwrap(),
                format!("\"{}\"", c.label())
            );
        }
    }
}
