//! The heuristic prompt classifier of `route` (no LLM): keywords in pt-BR and en, the prompt
//! length and file mentions.
//!
//! Keyword notation: a trailing `*` is a prefix (`implement*` matches `implementing` and
//! `implementar`); a keyword with a space is a whole-word phrase; anything else is an exact
//! word. Text is lower-cased and stripped of accents first, so `revisão` matches `revisao`.

use tokred_core::session::TaskClass;

/// The classifier's data.
#[derive(Debug, Clone)]
pub struct Rules {
    /// Prompts shorter than this (in characters) that mention no file are one-off tasks and
    /// are never worth delegating: they classify as [`TaskClass::Other`].
    pub min_prompt_chars: usize,
    /// Keywords per class.
    pub keywords: Vec<(TaskClass, Vec<&'static str>)>,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            min_prompt_chars: 30,
            keywords: vec![
                (
                    TaskClass::Architecture,
                    vec!["arquitet*", "architect*", "design*", "adr", "adrs"],
                ),
                (
                    TaskClass::Planning,
                    vec![
                        "planej*", "plano", "planos", "plan", "plans", "planning", "roadmap",
                    ],
                ),
                (
                    TaskClass::Implementation,
                    vec![
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
                (TaskClass::Review, vec!["revis*", "review*"]),
                (
                    TaskClass::Search,
                    vec![
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
                    vec![
                        "resumi*",
                        "resumo*",
                        "resuma*",
                        "summar*",
                        "tldr",
                        "recap",
                        "sintetiz*",
                    ],
                ),
            ],
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
#[must_use]
pub fn mentions_file(prompt: &str) -> bool {
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

/// Whether `word` (or, for a phrase, the padded text) matches `keyword`.
fn word_matches(keyword: &str, word: &str) -> bool {
    keyword
        .strip_suffix('*')
        .map_or_else(|| keyword == word, |stem| word.starts_with(stem))
}

/// Classifies a prompt.
///
/// The class with the most keyword hits wins; on a tie, the one whose keyword appears first
/// (`implement the plan` is implementation, `plan the implementation` is planning).
#[must_use]
pub fn classify(prompt: &str, rules: &Rules) -> TaskClass {
    if prompt.chars().count() < rules.min_prompt_chars && !mentions_file(prompt) {
        return TaskClass::Other;
    }
    let normalized = normalize(prompt);
    let words: Vec<&str> = normalized
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    // Padded, single-spaced text so that phrases match on word boundaries.
    let padded = format!(" {} ", words.join(" "));

    let mut best: Option<(TaskClass, usize, usize)> = None; // (class, hits, first position)
    for (class, keywords) in &rules.keywords {
        let mut hits = 0usize;
        let mut first = usize::MAX;
        for keyword in keywords {
            if keyword.contains(' ') {
                let needle = format!(" {keyword} ");
                for (position, _) in padded.match_indices(&needle) {
                    hits += 1;
                    first = first.min(position);
                }
            } else {
                let mut offset = 1; // the leading pad
                for word in &words {
                    if word_matches(keyword, word) {
                        hits += 1;
                        first = first.min(offset);
                    }
                    offset += word.len() + 1;
                }
            }
        }
        let better = hits > 0
            && best.is_none_or(|(_, best_hits, best_first)| {
                hits > best_hits || (hits == best_hits && first < best_first)
            });
        if better {
            best = Some((*class, hits, first));
        }
    }
    best.map_or(TaskClass::Other, |(class, _, _)| class)
}

/// The one-line `additionalContext` that suggests delegating a task of this class.
#[must_use]
pub fn suggestion(class: TaskClass) -> Option<String> {
    let (label, suffix) = match class {
        TaskClass::Architecture => ("architecture task", ""),
        TaskClass::Planning => ("planning task", ""),
        TaskClass::Implementation => ("implementation task", " if it involves more than one edit"),
        TaskClass::Review => ("review task", ""),
        TaskClass::Search => ("code search", " (read-only, cheap model)"),
        TaskClass::Summary => ("summarization task", ""),
        TaskClass::Other => return None,
    };
    let agent = class.agent()?;
    Some(format!(
        "tokred: {label} -> delegate to the {agent} subagent{suffix}."
    ))
}

#[cfg(test)]
mod tests {
    use TaskClass::{Architecture, Implementation, Other, Planning, Review, Search, Summary};

    use super::*;

    /// Table of real-looking prompts in pt-BR and en.
    const CASES: &[(&str, TaskClass)] = &[
        // architecture
        (
            "Qual a melhor arquitetura para o módulo de pagamentos?",
            Architecture,
        ),
        (
            "What is the best architecture for the payments module?",
            Architecture,
        ),
        (
            "Preciso fazer o design do serviço de notificações",
            Architecture,
        ),
        (
            "Design a event-driven approach for the billing service",
            Architecture,
        ),
        (
            "Discuta os trade-offs da arquitetura de filas",
            Architecture,
        ),
        // planning
        (
            "Planeje a migração do banco de dados para o Postgres 16",
            Planning,
        ),
        (
            "Preciso de um plano para dividir esse épico em tarefas",
            Planning,
        ),
        (
            "Plan the rollout of the new auth flow across all services",
            Planning,
        ),
        (
            "Help me with planning the next three sprints of the project",
            Planning,
        ),
        // implementation
        (
            "Implemente a paginação na listagem de usuários, por favor",
            Implementation,
        ),
        (
            "Corrija o bug de timezone no relatório mensal de vendas",
            Implementation,
        ),
        (
            "Refatore o serviço de cobrança para usar o novo cliente HTTP",
            Implementation,
        ),
        (
            "Implement pagination on the users list endpoint please",
            Implementation,
        ),
        (
            "Fix the failing test in src/billing/invoice.rs",
            Implementation,
        ),
        (
            "There is a bug in the checkout flow when the cart is empty",
            Implementation,
        ),
        (
            "Please refactor the config loader to avoid the global state",
            Implementation,
        ),
        // review
        (
            "Revise o código do PR de autenticação antes do merge",
            Review,
        ),
        ("Faça uma revisão dos testes do módulo de faturas", Review),
        (
            "Review the changes in the payments service before we merge",
            Review,
        ),
        (
            "Can you review src/auth/session.ts for security problems?",
            Review,
        ),
        // search
        (
            "Onde fica a validação do CPF no backend de cadastro?",
            Search,
        ),
        (
            "Encontre todos os usos da função legacy_charge no projeto",
            Search,
        ),
        (
            "Busque onde o token de acesso é renovado no cliente",
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
        // summary
        (
            "Resuma o conteúdo do documento de onboarding para o time",
            Summary,
        ),
        (
            "Faça um resumo dos logs de erro da última noite de deploy",
            Summary,
        ),
        (
            "Summarize the long output of the failing CI job for me",
            Summary,
        ),
        (
            "Give me a recap of what changed in docs/architecture.md",
            Summary,
        ),
        // ties and ordering
        (
            "Implement the plan we discussed yesterday for the importer",
            Implementation,
        ),
        (
            "Plan the implementation of the importer for next week",
            Planning,
        ),
        // short one-off prompts are never delegated
        ("fix the login bug", Other),
        ("corrija o bug", Other),
        ("onde fica o login?", Other),
        ("resuma isso", Other),
        ("ok", Other),
        ("", Other),
        // no keyword at all
        (
            "Obrigado, isso ficou ótimo e era exatamente o que eu queria",
            Other,
        ),
        (
            "What time zone does the scheduler run in by default today?",
            Other,
        ),
        // a short prompt that names a file is not one-off
        ("fix src/main.rs", Implementation),
        ("revisar utils.ts", Review),
        // accents and case do not matter
        (
            "REVISÃO do módulo de autenticação, com atenção à segurança",
            Review,
        ),
        (
            "ARQUITETURA do módulo de relatórios, por favor, avalie opções",
            Architecture,
        ),
    ];

    #[test]
    fn the_table_of_pt_br_and_en_prompts_classifies_as_expected() {
        let rules = Rules::default();
        for (prompt, expected) in CASES {
            assert_eq!(classify(prompt, &rules), *expected, "{prompt:?}");
        }
    }

    #[test]
    fn every_pt_br_accent_is_stripped() {
        assert_eq!(
            normalize("ÁÀÂÃÄ éèêë ÍÌÎÏ óòôõö úùûü ç"),
            "aaaaa eeee iiii ooooo uuuu c"
        );
    }

    #[test]
    fn the_most_hits_wins_over_the_first_keyword() {
        let rules = Rules::default();
        // One planning keyword first, then two implementation keywords.
        let prompt = "Plan: implement the importer and fix the retry bug in it";
        assert_eq!(classify(prompt, &rules), Implementation);
    }

    #[test]
    fn a_custom_threshold_changes_what_is_one_off() {
        let rules = Rules {
            min_prompt_chars: 5,
            ..Rules::default()
        };
        assert_eq!(classify("fix the login bug", &rules), Implementation);
    }

    #[test]
    fn file_mentions_are_detected() {
        for yes in [
            "look at src/main.rs",
            "edit main.rs now",
            "see C:\\work\\a.txt",
            "see ./x",
            "check package.json.",
            "open (lib.rs)",
        ] {
            assert!(mentions_file(yes), "{yes}");
        }
        for no in [
            "e.g. this thing",
            "version 3.14 is out",
            "end of sentence.",
            "hello world",
            "a.b",
            "",
            "file.toolongext",
            ".hidden",
        ] {
            assert!(!mentions_file(no), "{no}");
        }
    }

    #[test]
    fn suggestions_are_one_line_and_exist_only_for_actionable_classes() {
        for class in TaskClass::ALL {
            match suggestion(class) {
                Some(text) => {
                    assert!(!text.contains('\n'), "{text}");
                    assert!(text.starts_with("tokred: "), "{text}");
                    assert!(text.contains(class.agent().unwrap()), "{text}");
                }
                None => assert_eq!(class, Other),
            }
        }
        assert!(
            suggestion(Implementation)
                .unwrap()
                .contains("more than one edit")
        );
    }
}
