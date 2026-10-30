//! Static, curated content for the community/learning features.
//!
//! Every submodule exposes a `&'static` slice of plain data. Keep this crate
//! dependency-free and deterministic: it is compiled into the binary so the bot
//! works offline and answers instantly.

pub mod glossary;
pub mod mitre;
pub mod quiz;
pub mod resources;
pub mod roadmap;
pub mod tips;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Crypto,
    Web,
    Pwn,
    Reverse,
    Forensics,
    Network,
    Blue,
    Osint,
    Cloud,
    Misc,
}

impl Category {
    pub const ALL: &'static [Category] = &[
        Category::Crypto,
        Category::Web,
        Category::Pwn,
        Category::Reverse,
        Category::Forensics,
        Category::Network,
        Category::Blue,
        Category::Osint,
        Category::Cloud,
        Category::Misc,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Category::Crypto => "Cryptography",
            Category::Web => "Web",
            Category::Pwn => "Pwn",
            Category::Reverse => "Reverse Engineering",
            Category::Forensics => "Forensics",
            Category::Network => "Network",
            Category::Blue => "Blue Team",
            Category::Osint => "OSINT",
            Category::Cloud => "Cloud",
            Category::Misc => "Misc",
        }
    }

    pub const fn slug(self) -> &'static str {
        match self {
            Category::Crypto => "crypto",
            Category::Web => "web",
            Category::Pwn => "pwn",
            Category::Reverse => "reverse",
            Category::Forensics => "forensics",
            Category::Network => "network",
            Category::Blue => "blue",
            Category::Osint => "osint",
            Category::Cloud => "cloud",
            Category::Misc => "misc",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        let slug = slug.trim().to_ascii_lowercase();
        Self::ALL.iter().copied().find(|c| c.slug() == slug)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
}

impl Difficulty {
    pub const ALL: &'static [Difficulty] =
        &[Difficulty::Easy, Difficulty::Medium, Difficulty::Hard];

    pub const fn label(self) -> &'static str {
        match self {
            Difficulty::Easy => "Easy",
            Difficulty::Medium => "Medium",
            Difficulty::Hard => "Hard",
        }
    }

    pub const fn xp(self) -> u32 {
        match self {
            Difficulty::Easy => 10,
            Difficulty::Medium => 25,
            Difficulty::Hard => 50,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct QuizQuestion {
    pub id: &'static str,
    pub category: Category,
    pub difficulty: Difficulty,
    pub question: &'static str,
    /// Exactly four choices; `answer` is the index of the correct one.
    pub choices: [&'static str; 4],
    pub answer: usize,
    pub explanation: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct GlossaryEntry {
    pub term: &'static str,
    pub aliases: &'static [&'static str],
    pub definition: &'static str,
    pub category: Category,
}

#[derive(Clone, Copy, Debug)]
pub struct Tip {
    pub id: &'static str,
    pub text: &'static str,
    pub category: Category,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    Course,
    Book,
    Blog,
    Tool,
    Lab,
    Certification,
    Cheatsheet,
    Podcast,
}

impl ResourceKind {
    pub const fn label(self) -> &'static str {
        match self {
            ResourceKind::Course => "Course",
            ResourceKind::Book => "Book",
            ResourceKind::Blog => "Blog",
            ResourceKind::Tool => "Tool",
            ResourceKind::Lab => "Lab",
            ResourceKind::Certification => "Certification",
            ResourceKind::Cheatsheet => "Cheatsheet",
            ResourceKind::Podcast => "Podcast",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Resource {
    pub name: &'static str,
    pub url: &'static str,
    pub description: &'static str,
    pub kind: ResourceKind,
    pub free: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Track {
    Web,
    Pwn,
    Crypto,
    Forensics,
    BlueTeam,
    RedTeam,
    Cloud,
    Osint,
}

impl Track {
    pub const ALL: &'static [Track] = &[
        Track::Web,
        Track::Pwn,
        Track::Crypto,
        Track::Forensics,
        Track::BlueTeam,
        Track::RedTeam,
        Track::Cloud,
        Track::Osint,
    ];

    pub const fn slug(self) -> &'static str {
        match self {
            Track::Web => "web",
            Track::Pwn => "pwn",
            Track::Crypto => "crypto",
            Track::Forensics => "forensics",
            Track::BlueTeam => "blue",
            Track::RedTeam => "red",
            Track::Cloud => "cloud",
            Track::Osint => "osint",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Track::Web => "Web Exploitation",
            Track::Pwn => "Binary Exploitation",
            Track::Crypto => "Cryptography",
            Track::Forensics => "Digital Forensics",
            Track::BlueTeam => "Blue Team / DFIR",
            Track::RedTeam => "Red Team / Pentest",
            Track::Cloud => "Cloud Security",
            Track::Osint => "OSINT",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        let slug = slug.trim().to_ascii_lowercase();
        Self::ALL.iter().copied().find(|t| t.slug() == slug)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RoadmapStep {
    pub track: Track,
    /// 1-based ordering inside the track.
    pub order: u8,
    pub title: &'static str,
    pub description: &'static str,
    /// Resource names that must exist in [`resources::RESOURCES`].
    pub resources: &'static [&'static str],
}

#[derive(Clone, Copy, Debug)]
pub struct Technique {
    pub id: &'static str,
    pub name: &'static str,
    pub tactic: &'static str,
    pub description: &'static str,
    pub detection: &'static str,
}

/// Find a quiz question by id (case-insensitive).
pub fn quiz_by_id(id: &str) -> Option<&'static QuizQuestion> {
    let id = id.trim();
    quiz::QUESTIONS
        .iter()
        .find(|q| q.id.eq_ignore_ascii_case(id))
}

/// Case-insensitive substring search across terms and aliases.
pub fn search_glossary(query: &str, limit: usize) -> Vec<&'static GlossaryEntry> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    glossary::GLOSSARY
        .iter()
        .filter(|entry| {
            entry.term.to_ascii_lowercase().contains(&needle)
                || entry
                    .aliases
                    .iter()
                    .any(|a| a.to_ascii_lowercase().contains(&needle))
                || entry.definition.to_ascii_lowercase().contains(&needle)
        })
        .take(limit)
        .collect()
}

/// Case-insensitive lookup by technique id (`T1059`) or name substring.
pub fn techniques_matching(query: &str, limit: usize) -> Vec<&'static Technique> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    mitre::TECHNIQUES
        .iter()
        .filter(|t| {
            t.id.to_ascii_lowercase().contains(&needle)
                || t.name.to_ascii_lowercase().contains(&needle)
                || t.tactic.to_ascii_lowercase().contains(&needle)
        })
        .take(limit)
        .collect()
}

pub fn steps_for_track(track: Track) -> Vec<&'static RoadmapStep> {
    let mut steps: Vec<_> = roadmap::STEPS.iter().filter(|s| s.track == track).collect();
    steps.sort_by_key(|s| s.order);
    steps
}

/// Deterministic daily rotation (UTC day number as seed).
pub fn tip_of_day(day_seed: u64) -> &'static Tip {
    let idx = (day_seed as usize) % tips::TIPS.len().max(1);
    &tips::TIPS[idx]
}

/// Deterministic daily rotation (UTC day number as seed).
pub fn quiz_of_day(day_seed: u64) -> &'static QuizQuestion {
    let idx = (day_seed as usize) % quiz::QUESTIONS.len().max(1);
    &quiz::QUESTIONS[idx]
}
