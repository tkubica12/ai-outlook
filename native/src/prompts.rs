use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Prompts {
    pub preparation: String,
    pub recap: String,
    pub quick: String,
    pub thorough: String,
    pub context: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Preparation,
    Recap,
    Quick,
    Thorough,
    Context,
}

impl Kind {
    pub const ALL: [Self; 5] = [Self::Preparation, Self::Recap, Self::Quick, Self::Thorough, Self::Context];

    pub fn label(self) -> &'static str {
        match self {
            Self::Preparation => "Before the meeting",
            Self::Recap => "After the meeting",
            Self::Quick => "Quick answers",
            Self::Thorough => "Thorough research",
            Self::Context => "Workplace research",
        }
    }

    pub fn default_text(self) -> &'static str {
        match self {
            Self::Preparation => "Prioritize concrete preparation for my role, relevant technical context, existing reusable materials, previous commitments and questions worth resolving before the meeting. Avoid generic tips.",
            Self::Recap => "Prioritize every mention of me, what was said about me, and especially tasks or commitments assigned to me (owner, deliverable and deadline when stated). Highlight technical details, AI, Azure AI Foundry, architecture, demos and practical takeaways. Include decisions, shared materials and original links. Omit corporate filler. Distinguish an explicit assignment from a suggestion or inference.",
            Self::Quick => "Answer directly and concisely. Prefer the most relevant facts and a clear next action over long explanations.",
            Self::Thorough => "Investigate the actual question in depth. Connect technical mechanisms, useful evidence and practical tradeoffs. Separate verified facts, interpretations and gaps.",
            Self::Context => "Prioritize relevant recent mail, Teams discussions, earlier meetings, decisions, open commitments and shared materials. Search both the customer/person and technical topic. Prefer original sources.",
        }
    }
}

impl Default for Prompts {
    fn default() -> Self {
        Self {
            preparation: Kind::Preparation.default_text().into(),
            recap: Kind::Recap.default_text().into(),
            quick: Kind::Quick.default_text().into(),
            thorough: Kind::Thorough.default_text().into(),
            context: Kind::Context.default_text().into(),
        }
    }
}

impl Prompts {
    pub fn text(&self, kind: Kind) -> &str {
        match kind {
            Kind::Preparation => &self.preparation,
            Kind::Recap => &self.recap,
            Kind::Quick => &self.quick,
            Kind::Thorough => &self.thorough,
            Kind::Context => &self.context,
        }
    }

    pub fn edit(&mut self, kind: Kind) -> &mut String {
        match kind {
            Kind::Preparation => &mut self.preparation,
            Kind::Recap => &mut self.recap,
            Kind::Quick => &mut self.quick,
            Kind::Thorough => &mut self.thorough,
            Kind::Context => &mut self.context,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        for kind in Kind::ALL {
            let text = self.text(kind);
            if text.trim().is_empty() || text.len() > 4000
                || text.chars().any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
            {
                return Err("Each prompt needs 1 to 4000 UTF-8 bytes and no unsupported control characters".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_migrate_restore_and_reject_invalid_edits() {
        let mut prompts: Prompts = serde_json::from_str("{}").unwrap();
        assert_eq!(prompts, Prompts::default());
        prompts.validate().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let mut preferences = crate::storage::Preferences::default();
        preferences.prompts.recap = "My persisted guidance".into();
        crate::storage::Store::open(directory.path()).unwrap().save_preferences(&preferences).unwrap();
        assert_eq!(crate::storage::Store::open(directory.path()).unwrap().preferences().unwrap().prompts, preferences.prompts);
        *prompts.edit(Kind::Recap) = "Technical outcomes".into();
        let saved = serde_json::to_string(&prompts).unwrap();
        assert_eq!(serde_json::from_str::<Prompts>(&saved).unwrap(), prompts);
        *prompts.edit(Kind::Recap) = Kind::Recap.default_text().into();
        assert_eq!(prompts, Prompts::default());
        prompts.quick.clear();
        assert!(prompts.validate().is_err());
    }
}
