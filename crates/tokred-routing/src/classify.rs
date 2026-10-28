//! Classification in layers (spec 5.2). Each layer returns a [`Classification`]: the task
//! class, a confidence, the layer and a reason. The keyword data is the `route` hook's
//! (pt-BR and en), with hit counts kept so the confidence can be graded.
//!
//! Keyword notation: a trailing `*` is a prefix (`implement*` matches `implementing` and
//! `implementar`); a keyword with a space is a whole-word phrase; anything else is an exact
//! word. Text is lower-cased and stripped of accents first, so `revisão` matches `revisao`.

use serde_json::Value;
use tokred_core::config::{RoutingTiers, TierChoice};
use tokred_core::session::TaskClass;

/// The class of a request, as routing sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteClass {
    /// Locating code (cheap by default).
    Search,
    /// Summarizing (cheap by default).
    Summary,
    /// Implementing, fixing, refactoring (mid by default).
    Implementation,
    /// Planning (kept by default).
    Planning,
    /// Architecture and design (kept by default).
    Architecture,
    /// Reviewing (kept by default).
    Review,
    /// A side request such as the session title (cheap by default).
    Auxiliary,
    /// No signal: always kept.
    Other,
}

impl RouteClass {
    /// Value stored in `routing_decisions.task_class`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Summary => "summary",
            Self::Implementation => "implementation",
            Self::Planning => "planning",
            Self::Architecture => "architecture",
            Self::Review => "review",
            Self::Auxiliary => "auxiliary",
            Self::Other => "other",
        }
    }

    /// The configured tier for this class.
    #[must_use]
    pub const fn tier(self, tiers: &RoutingTiers) -> TierChoice {
        match self {
            Self::Search => tiers.search,
            Self::Summary => tiers.summary,
            Self::Implementation => tiers.implementation,
            Self::Planning => tiers.planning,
            Self::Architecture => tiers.architecture,
            Self::Review => tiers.review,
            Self::Auxiliary => tiers.auxiliary,
            Self::Other => TierChoice::Keep,
        }
    }

    const fn from_task(class: TaskClass) -> Self {
        match class {
            TaskClass::Architecture => Self::Architecture,
            TaskClass::Planning => Self::Planning,
            TaskClass::Implementation => Self::Implementation,
            TaskClass::Review => Self::Review,
            TaskClass::Search => Self::Search,
            TaskClass::Summary => Self::Summary,
            TaskClass::Other => Self::Other,
        }
    }

    const fn is_cheap_work(self) -> bool {
        matches!(self, Self::Search | Self::Summary)
    }
}

/// The layer that decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// `x-claude-code-agent-type` / `x-claude-code-request-class`.
    Headers,
    /// The subagent's model differs from the main loop's: chosen on purpose.
    Explicit,
    /// The tool set of the request.
    Tools,
    /// Keywords of the subagent's first message.
    Keywords,
    /// The auxiliary shape (no tools, no thinking, one message).
    Shape,
    /// An agent first seen mid-run: its model is kept.
    MidRun,
    /// No signal.
    Default,
}

impl Layer {
    /// Value stored in `routing_decisions.layer`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Headers => "headers",
            Self::Explicit => "explicit",
            Self::Tools => "tools",
            Self::Keywords => "keywords",
            Self::Shape => "shape",
            Self::MidRun => "mid_run",
            Self::Default => "default",
        }
    }
}

/// The result of classifying a request.
#[derive(Debug, Clone, PartialEq)]
pub struct Classification {
    /// The task class.
    pub class: RouteClass,
    /// Confidence, 0..=1. Routing acts only at or above `min_confidence`.
    pub confidence: f64,
    /// The layer that decided.
    pub layer: Layer,
    /// Why.
    pub reason: String,
}

impl Classification {
    pub(crate) fn new(
        class: RouteClass,
        confidence: f64,
        layer: Layer,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            class,
            confidence,
            layer,
            reason: reason.into(),
        }
    }
}

/// Tools that change nothing (spec 5.2, layer 3). Anything else raises the risk.
const READ_ONLY_TOOLS: &[&str] = &[
    "Read",
    "Grep",
    "Glob",
    "LS",
    "NotebookRead",
    "WebFetch",
    "WebSearch",
    "TodoWrite",
    "ListMcpResourcesTool",
    "ReadMcpResourceTool",
];

/// Classifies a subagent's first request: headers, tools, keywords, default. The explicit
/// choice guard runs before this (see the policy).
#[must_use]
pub fn classify_subagent(
    agent_type: Option<&str>,
    tools: &[String],
    prompt: &str,
) -> Classification {
    if let Some(found) = agent_type.and_then(header_class) {
        return found;
    }
    let risky: Vec<&str> = tools
        .iter()
        .map(String::as_str)
        .filter(|tool| !READ_ONLY_TOOLS.contains(tool))
        .collect();
    let keywords = keyword_class(prompt);
    let read_only = risky.is_empty() && !tools.is_empty();
    if read_only {
        let shown = list(tools.iter().map(String::as_str));
        match keywords {
            None => {
                return Classification::new(
                    RouteClass::Search,
                    0.85,
                    Layer::Tools,
                    format!("read-only tools only ({shown})"),
                );
            }
            Some(hit) if hit.class.is_cheap_work() => {
                return Classification::new(
                    hit.class,
                    0.9,
                    Layer::Tools,
                    format!(
                        "read-only tools only ({shown}) and {} keywords",
                        hit.class.as_str()
                    ),
                );
            }
            // A read-only planner or architect: the keywords decide.
            Some(_) => {}
        }
    }
    if let Some(hit) = keywords {
        let mut confidence = hit.confidence();
        let mut reason = format!(
            "{} keyword hits for {} (runner-up {})",
            hit.hits,
            hit.class.as_str(),
            hit.runner_up
        );
        if hit.class.is_cheap_work() && !risky.is_empty() {
            confidence -= 0.2;
            reason = format!("{reason}; tools that change things: {}", list(risky));
        }
        return Classification::new(hit.class, confidence, Layer::Keywords, reason);
    }
    Classification::new(
        RouteClass::Other,
        0.0,
        Layer::Default,
        "no classification signal",
    )
}

/// Classifies an auxiliary request: by header when Claude Code sent one, else by its shape.
#[must_use]
pub fn classify_auxiliary(request_class: Option<&str>) -> Classification {
    if request_class.is_some_and(|class| class.eq_ignore_ascii_case("auxiliary")) {
        return Classification::new(
            RouteClass::Auxiliary,
            1.0,
            Layer::Headers,
            "request-class auxiliary",
        );
    }
    Classification::new(
        RouteClass::Auxiliary,
        0.9,
        Layer::Shape,
        "no tools, no thinking and one message",
    )
}

fn header_class(agent_type: &str) -> Option<Classification> {
    let class = match agent_type.to_ascii_lowercase().as_str() {
        "explore" => RouteClass::Search,
        "plan" => RouteClass::Planning,
        _ => return None,
    };
    Some(Classification::new(
        class,
        0.95,
        Layer::Headers,
        format!("agent-type {agent_type}"),
    ))
}

fn list<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    items.into_iter().collect::<Vec<_>>().join(", ")
}

/// The text of the first user message, without the `<system-reminder>` blocks Claude Code
/// adds (they carry project instructions, not the task).
#[must_use]
pub fn first_user_text(body: &Value) -> String {
    let message = body
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| {
            messages
                .iter()
                .find(|m| m.get("role").and_then(Value::as_str) == Some("user"))
        });
    let text = match message.and_then(|m| m.get("content")) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    strip_reminders(&text)
}

fn strip_reminders(text: &str) -> String {
    const OPEN: &str = "<system-reminder>";
    const CLOSE: &str = "</system-reminder>";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        out.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len()..];
        rest = after
            .find(CLOSE)
            .map_or("", |end| &after[end + CLOSE.len()..]);
    }
    out.push_str(rest);
    out
}

/// The `route` hook's keyword data (pt-BR and en).
const KEYWORDS: &[(TaskClass, &[&str])] = &[
    (
        TaskClass::Architecture,
        &["arquitet*", "architect*", "design*", "adr", "adrs"],
    ),
    (
        TaskClass::Planning,
        &[
            "planej*", "plano", "planos", "plan", "plans", "planning", "roadmap",
        ],
    ),
    (
        TaskClass::Implementation,
        &[
            "implement*",
            "develop*",
            "desenvolv*",
            "corrig*",
            "corrij*",
            "consert*",
            "conserta*",
            "fix",
            "fixes",
            "fixed",
            "fixing",
            "bug",
            "bugs",
            "refator*",
            "refactor*",
            "adicion*",
            "add",
            "crie",
            "criar",
            "create",
        ],
    ),
    (TaskClass::Review, &["revis*", "review*"]),
    (
        TaskClass::Search,
        &[
            "onde fica",
            "onde esta",
            "where is",
            "where are",
            "where does",
            "find*",
            "locate",
            "search*",
            "encontr*",
            "busc*",
            "busq*",
            "procur*",
            "localiz*",
            "pesquis*",
            "grep",
        ],
    ),
    (
        TaskClass::Summary,
        &[
            "resumi*",
            "resumo*",
            "resuma*",
            "summar*",
            "tldr",
            "recap",
            "sintetiz*",
        ],
    ),
];

/// Prompts shorter than this that mention no file are one-off: no keyword class.
const MIN_PROMPT_CHARS: usize = 30;

/// The winning keyword class and how clear the win was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeywordHit {
    class: RouteClass,
    hits: usize,
    runner_up: usize,
}

impl KeywordHit {
    /// Uncontested wins are trusted; a close race is not.
    fn confidence(self) -> f64 {
        if self.runner_up == 0 {
            if self.hits >= 2 { 0.9 } else { 0.8 }
        } else if self.hits >= self.runner_up + 2 {
            0.8
        } else {
            0.6
        }
    }
}

/// Lower-cases and strips the accents that pt-BR uses.
fn normalize(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

/// Whether the prompt names a file: a path separator, or a word ending in `.ext`.
fn mentions_file(prompt: &str) -> bool {
    prompt.split_whitespace().any(|token| {
        let token = token.trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '\\');
        if token.contains('/') || token.contains('\\') {
            return true;
        }
        token.rsplit_once('.').is_some_and(|(stem, ext)| {
            !stem.is_empty()
                && (2..=5).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && ext.chars().any(|c| c.is_ascii_alphabetic())
        })
    })
}

fn word_matches(keyword: &str, word: &str) -> bool {
    keyword
        .strip_suffix('*')
        .map_or_else(|| keyword == word, |stem| word.starts_with(stem))
}

/// The class with the most keyword hits; on a tie, the one whose keyword appears first.
fn keyword_class(prompt: &str) -> Option<KeywordHit> {
    if prompt.chars().count() < MIN_PROMPT_CHARS && !mentions_file(prompt) {
        return None;
    }
    let normalized = normalize(prompt);
    let words: Vec<&str> = normalized
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let padded = format!(" {} ", words.join(" "));

    // (class, hits, first position)
    let mut scores: Vec<(TaskClass, usize, usize)> = Vec::new();
    for (class, keywords) in KEYWORDS {
        let mut hits = 0usize;
        let mut first = usize::MAX;
        for keyword in *keywords {
            if keyword.contains(' ') {
                for (position, _) in padded.match_indices(&format!(" {keyword} ")) {
                    hits += 1;
                    first = first.min(position);
                }
            } else {
                let mut offset = 1;
                for word in &words {
                    if word_matches(keyword, word) {
                        hits += 1;
                        first = first.min(offset);
                    }
                    offset += word.len() + 1;
                }
            }
        }
        if hits > 0 {
            scores.push((*class, hits, first));
        }
    }
    scores.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
    let (class, hits, _) = *scores.first()?;
    Some(KeywordHit {
        class: RouteClass::from_task(class),
        hits,
        runner_up: scores.get(1).map_or(0, |s| s.1),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use RouteClass::{Architecture, Implementation, Other, Planning, Review, Search, Summary};

    /// Subagent task prompts in pt-BR and en, with the class the keywords give.
    const CASES: &[(&str, RouteClass)] = &[
        (
            "Qual a melhor arquitetura para o módulo de pagamentos?",
            Architecture,
        ),
        (
            "Design an event-driven approach for the billing service",
            Architecture,
        ),
        (
            "Discuta os trade-offs da arquitetura de filas",
            Architecture,
        ),
        (
            "Planeje a migração do banco de dados para o Postgres 16",
            Planning,
        ),
        (
            "Plan the rollout of the new auth flow across all services",
            Planning,
        ),
        (
            "Implemente a paginação na listagem de usuários, por favor",
            Implementation,
        ),
        (
            "Corrija o bug de timezone no relatório mensal de vendas",
            Implementation,
        ),
        (
            "Fix the failing test in src/billing/invoice.rs",
            Implementation,
        ),
        (
            "Please refactor the config loader to avoid the global state",
            Implementation,
        ),
        (
            "Revise o código do PR de autenticação antes do merge",
            Review,
        ),
        (
            "Review the changes in the payments service before we merge",
            Review,
        ),
        (
            "Onde fica a validação do CPF no backend de cadastro?",
            Search,
        ),
        (
            "Encontre todos os usos da função legacy_charge no projeto",
            Search,
        ),
        (
            "Where is the retry logic for failed webhook deliveries?",
            Search,
        ),
        (
            "Find every place that calls the old pricing function",
            Search,
        ),
        (
            "Resuma o conteúdo do documento de onboarding para o time",
            Summary,
        ),
        (
            "Summarize the long output of the failing CI job for me",
            Summary,
        ),
        (
            "Implement the plan we discussed yesterday for the importer",
            Implementation,
        ),
        (
            "Plan the implementation of the importer for next week",
            Planning,
        ),
        ("fix the login bug", Other),
        ("onde fica o login?", Other),
        ("", Other),
        (
            "Obrigado, isso ficou ótimo e era exatamente o que eu queria",
            Other,
        ),
        ("revisar utils.ts", Review),
        (
            "REVISÃO do módulo de autenticação, com atenção à segurança",
            Review,
        ),
    ];

    fn class_of(prompt: &str) -> RouteClass {
        keyword_class(prompt).map_or(Other, |hit| hit.class)
    }

    #[test]
    fn the_table_of_pt_br_and_en_prompts_classifies_as_expected() {
        for (prompt, expected) in CASES {
            assert_eq!(class_of(prompt), *expected, "{prompt:?}");
        }
    }

    #[test]
    fn confidence_grows_with_a_clear_win() {
        let hit = |hits, runner_up| KeywordHit {
            class: Search,
            hits,
            runner_up,
        };
        assert!((hit(1, 0).confidence() - 0.8).abs() < 1e-9);
        assert!((hit(2, 0).confidence() - 0.9).abs() < 1e-9);
        assert!((hit(3, 1).confidence() - 0.8).abs() < 1e-9);
        assert!((hit(2, 1).confidence() - 0.6).abs() < 1e-9);
        let contested =
            keyword_class("Plan: implement the importer and fix the retry bug in it").unwrap();
        assert_eq!(contested.class, Implementation);
        assert_eq!(contested.runner_up, 1);
    }

    fn tools(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn headers_decide_first() {
        let explore = classify_subagent(Some("Explore"), &tools(&["Bash"]), "implement it all");
        assert_eq!((explore.class, explore.layer), (Search, Layer::Headers));
        let plan = classify_subagent(Some("plan"), &[], "");
        assert_eq!(plan.class, Planning);
        let custom = classify_subagent(Some("custom"), &[], "");
        assert_eq!(custom.layer, Layer::Default);
    }

    #[test]
    fn read_only_tools_mean_search_unless_the_prompt_asks_for_more() {
        let read = tools(&["Read", "Grep", "Glob"]);
        let plain = classify_subagent(None, &read, "Tell me how sessions expire in this app");
        assert_eq!((plain.class, plain.layer), (Search, Layer::Tools));
        assert!(plain.reason.contains("Read, Grep, Glob"));
        let summary = classify_subagent(None, &read, "Resuma os logs de erro do último deploy");
        assert_eq!((summary.class, summary.layer), (Summary, Layer::Tools));
        assert!((summary.confidence - 0.9).abs() < 1e-9);
        let planner = classify_subagent(
            None,
            &read,
            "Planeje a migração do banco de dados para o Postgres 16",
        );
        assert_eq!((planner.class, planner.layer), (Planning, Layer::Keywords));
    }

    #[test]
    fn tools_that_change_things_lower_the_confidence_of_cheap_work() {
        let writer = tools(&["Read", "Edit", "Bash"]);
        let search = classify_subagent(
            None,
            &writer,
            "Find every place that calls the old pricing function",
        );
        assert_eq!((search.class, search.layer), (Search, Layer::Keywords));
        assert!((search.confidence - 0.6).abs() < 1e-9);
        assert!(search.reason.contains("Edit, Bash"));
        let implement = classify_subagent(
            None,
            &writer,
            "Implement pagination on the users list endpoint please",
        );
        assert!((implement.confidence - 0.8).abs() < 1e-9);
        let nothing = classify_subagent(None, &writer, "ok");
        assert_eq!((nothing.class, nothing.layer), (Other, Layer::Default));
        let no_tools = classify_subagent(None, &[], "Tell me how sessions expire in this app");
        assert_eq!(no_tools.layer, Layer::Default);
    }

    #[test]
    fn auxiliary_requests_are_classified_by_header_or_shape() {
        let header = classify_auxiliary(Some("Auxiliary"));
        assert_eq!(
            (header.class, header.layer),
            (RouteClass::Auxiliary, Layer::Headers)
        );
        let shape = classify_auxiliary(None);
        assert_eq!(shape.layer, Layer::Shape);
        assert!((shape.confidence - 0.9).abs() < 1e-9);
        assert_eq!(classify_auxiliary(Some("main")).layer, Layer::Shape);
    }

    #[test]
    fn the_first_user_text_skips_system_reminders() {
        let body = json!({"messages": [
            {"role": "system", "content": "ignored"},
            {"role": "user", "content": [
                {"type": "text", "text": "<system-reminder>architecture rules</system-reminder>"},
                {"type": "image"},
                {"type": "text", "text": "find the parser<system-reminder>plan"}
            ]}
        ]});
        assert_eq!(first_user_text(&body), "\nfind the parser");
        let plain = json!({"messages": [{"role": "user", "content": "a <system-reminder>x</system-reminder>b"}]});
        assert_eq!(first_user_text(&plain), "a b");
        assert_eq!(
            first_user_text(&json!({"messages": [{"role": "user", "content": 1}]})),
            ""
        );
        assert_eq!(first_user_text(&json!({})), "");
    }

    #[test]
    fn names_and_tiers() {
        let tiers = RoutingTiers::default();
        let all = [
            (Search, "search", TierChoice::Cheap),
            (Summary, "summary", TierChoice::Cheap),
            (Implementation, "implementation", TierChoice::Mid),
            (Planning, "planning", TierChoice::Keep),
            (Architecture, "architecture", TierChoice::Keep),
            (Review, "review", TierChoice::Keep),
            (RouteClass::Auxiliary, "auxiliary", TierChoice::Cheap),
            (Other, "other", TierChoice::Keep),
        ];
        for (class, name, tier) in all {
            assert_eq!(class.as_str(), name);
            assert_eq!(class.tier(&tiers), tier);
        }
        for (layer, name) in [
            (Layer::Headers, "headers"),
            (Layer::Explicit, "explicit"),
            (Layer::Tools, "tools"),
            (Layer::Keywords, "keywords"),
            (Layer::Shape, "shape"),
            (Layer::MidRun, "mid_run"),
            (Layer::Default, "default"),
        ] {
            assert_eq!(layer.as_str(), name);
        }
        assert_eq!(RouteClass::from_task(TaskClass::Other), Other);
    }

    #[test]
    fn every_pt_br_accent_is_stripped_and_files_are_detected() {
        assert_eq!(
            normalize("ÁÀÂÃÄ éèêë ÍÌÎÏ óòôõö úùûü ç"),
            "aaaaa eeee iiii ooooo uuuu c"
        );
        assert!(mentions_file("see C:\\work\\a.txt"));
        assert!(mentions_file("open (lib.rs)"));
        assert!(!mentions_file("version 3.14 is out"));
    }
}
