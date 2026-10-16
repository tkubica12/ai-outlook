use std::{collections::BTreeSet, fs::OpenOptions, io::Write, path::PathBuf, time::Duration};
use tomlook::{
    chat::{History, Mode},
    decision::Router,
};

const REPORTED_QUESTION: &str = "Pro kterou schůzku budu potřebovat použít eh eh demo z eh eh autopilots, který mám v rámci GitHubu? Agent demos";

#[derive(Clone, Copy, Debug)]
enum Context {
    None,
    Research,
    Verified,
    Unverified,
}

struct Case {
    id: &'static str,
    question: &'static str,
    expected: Mode,
    context: Context,
    meeting: Option<&'static str>,
}

fn case(id: &'static str, question: &'static str, expected: Mode) -> Case {
    Case {
        id,
        question,
        expected,
        context: Context::None,
        meeting: None,
    }
}

fn cases() -> Vec<Case> {
    let fast = |id, question| case(id, question, Mode::Fast);
    let deep = |id, question| case(id, question, Mode::Deep);
    vec![
        fast("next-meeting", "Kdy se příště potkám s North Star?"),
        fast(
            "workshop-place",
            "Ukaž místo a odkaz na nejbližší workshop.",
        ),
        fast(
            "last-confirmation",
            "Dohledej poslední e-mail od Jamieho, který potvrzuje termín.",
        ),
        fast(
            "month-list",
            "Vypiš názvy a časy mých schůzek za následujících 30 dní.",
        ),
        fast(
            "demo-readme",
            "Co obsahuje README dema autopilots-on-azure v agent-demos?",
        ),
        fast(
            "demo-files",
            "Vypiš soubory ve složce invoice-pilot v mém repozitáři.",
        ),
        fast("named-kickoff", "Kdy začíná North Star Hackathon Kick-off?"),
        fast(
            "literal-topic",
            "Najdi schůzky se slovem Autopilot v názvu.",
        ),
        fast("organizer", "Kdo organizuje North Star Hackathon Kick-off?"),
        fast(
            "explicit-mail",
            "Shrň, o co mě Jamie žádá v posledním e-mailu ve vlákně North Star Hackathon.",
        ),
        fast(
            "repo-commit",
            "Jaká je poslední změna v repozitáři agent-demos?",
        ),
        fast(
            "existing-insights",
            "Stručně shrň uložené Insights k North Star kick-offu. Nic dalšího neověřuj.",
        ),
        Case {
            meeting: Some("Europe North Learning ON AIR: Copilot to Autopilot"),
            ..fast("selected-attendees", "Ukaž účastníky této schůzky.")
        },
        fast(
            "targeted-teams",
            "Najdi poslední zprávu od Rileyho v chatu North Star s odkazem na demo.",
        ),
        fast(
            "upcoming-hackathons",
            "Vypiš moje nejbližší tři schůzky, které mají v názvu hackathon.",
        ),
        Case {
            context: Context::Research,
            ..fast(
                "explain-uncertainty",
                "Co přesně znamená neověřená připravenost dema v předchozí odpovědi?",
            )
        },
        Case {
            context: Context::Research,
            ..fast(
                "repeat-first-date",
                "Z těch tří už zpracovaných schůzek ukaž jen datum té první.",
            )
        },
        Case {
            context: Context::Verified,
            ..fast(
                "repeat-verified-customer",
                "Tak jen zopakuj, pro koho je ta ukázka, bez dalšího hledání.",
            )
        },
        Case {
            context: Context::Verified,
            ..fast(
                "repeat-verified-repo",
                "Pošli mi jen název toho už ověřeného repozitáře.",
            )
        },
        fast(
            "brief-direct-lookup",
            "Jednou větou: kdy a kde je North Star kick-off?",
        ),
        deep("reported-demo-association", REPORTED_QUESTION),
        deep(
            "short-demo-customer",
            "Komu mám ukazovat tu ukázku invoice-pilot z GitHubu?",
        ),
        deep(
            "artifact-meeting-link",
            "K jakému zákaznickému jednání patří projekt invoice-pilot, který mám v repozitáři?",
        ),
        deep(
            "promised-demo",
            "Kde jsem slíbil předvést ukázku z mého repozitáře?",
        ),
        deep(
            "presenter-versus-listener",
            "U kterých autopilot workshopů mám prezentovat já a na kterých jen poslouchám?",
        ),
        Case {
            meeting: Some("Europe North Learning ON AIR: Copilot to Autopilot"),
            ..deep(
                "selected-role",
                "Mám na této akci vést demo, nebo jsem jen posluchač?",
            )
        },
        Case {
            context: Context::Unverified,
            ..deep(
                "unverified-followup",
                "Tak na kterém jednání budu tu ukázku potřebovat?",
            )
        },
        Case {
            meeting: Some("North Star workshop"),
            ..deep(
                "selected-expectations",
                "Co mám na tuhle schůzku vlastně připravit já?",
            )
        },
        deep(
            "single-demo-readiness",
            "Je agent z mého repozitáře připravený na pondělní zákaznickou schůzku?",
        ),
        deep(
            "conflicting-evidence",
            "V kalendáři je školení, ale v e-mailu se píše o mém demu. Zjisti, na kterou akci se ta domluva vztahuje.",
        ),
        deep(
            "monthly-readiness",
            "Zjisti, kde mám příští měsíc prezentovat nebo dělat demo a co ještě nemám hotové. Ověř kalendář, poštu, Teams, místní prezentace i repozitáře a navrhni plán přípravy.",
        ),
        deep(
            "architecture-comparison",
            "Připrav podrobný návrh zákaznického workshopu: porovnej tři architektury, ověř zdroje a zohledni historii našich jednání.",
        ),
        Case {
            context: Context::Research,
            ..deep(
                "continue-investigation",
                "Pokračuj průzkumem zbývajících pěti schůzek a dopracuj plán přípravy.",
            )
        },
        deep(
            "english-investigation",
            "Investigate the customer's prior commitments across mail and Teams, compare available demos and produce a detailed preparation plan.",
        ),
        deep(
            "spoken-association",
            "Hele a kde budu vlastně ten svůj GitHub demo projekt ukazovat, ne jen někde poslouchat prezentaci?",
        ),
        deep(
            "not-my-training",
            "Nemyslím školení pro mě. Pro kterého zákazníka jsem chystal to demo z agent-demos?",
        ),
        deep(
            "brief-but-investigative",
            "Jednou větou: kde ode mě čekají demo z invoice-pilot?",
        ),
        deep(
            "english-role-association",
            "Which upcoming meeting actually expects me to present this repo's demo?",
        ),
        Case {
            context: Context::Unverified,
            ..deep(
                "challenge-topic-match",
                "Podobný název nestačí. Ověř, komu jsem slíbil to demo a při jaké příležitosti.",
            )
        },
        deep(
            "inferred-commitment",
            "Mám už hotovou prezentaci. Na které z příštích schůzek ji po mně budou chtít?",
        ),
    ]
}

fn holdouts() -> Vec<Case> {
    let fast = |id, question| case(id, question, Mode::Fast);
    let deep = |id, question| case(id, question, Mode::Deep);
    vec![
        fast(
            "holdout-connection",
            "Zítra se mám potkat s Lumen Labs. V kolik to začíná a jaký je odkaz na připojení?",
        ),
        fast(
            "holdout-organizer-field",
            "Kdo je v pozvánce na Partner Day uvedený jako organizátor?",
        ),
        fast(
            "holdout-file-paths",
            "Jen mi vypiš cesty k HTML souborům ve složce presentations v tom repozitáři.",
        ),
        fast(
            "holdout-quote-request",
            "Odcituj mi pasáž z poslední zprávy od Adama v tomhle chatu, kde žádá o ukázku.",
        ),
        Case {
            context: Context::Unverified,
            ..fast(
                "holdout-explain-only",
                "Vysvětli jen slovo neověřeno z tvojí odpovědi; nic nedohledávej.",
            )
        },
        Case {
            context: Context::Verified,
            ..fast(
                "holdout-established-client",
                "Který zákazník už byl v předchozím výsledku doložený? Jen jméno.",
            )
        },
        deep(
            "holdout-notebook-promise",
            "Mám tu notebook revenue-forecast. Komu jsem slíbil, že ho předvedu, a do kdy?",
        ),
        deep(
            "holdout-next-meeting-duty",
            "Mám na příští poradu něco vyrábět, nebo mi stačí přijít?",
        ),
        Case {
            meeting: Some("Data Community Meetup"),
            ..deep(
                "holdout-talk-or-training",
                "Tohle je pro mě školení, nebo mám dodat vlastní vystoupení?",
            )
        },
        deep(
            "holdout-presentation-evidence",
            "Z těch materiálů v OneDrivu mi zjisti, co patří k páteční prezentaci a kde mi ještě chybí podklady.",
        ),
        Case {
            context: Context::Unverified,
            ..deep(
                "holdout-prove-association",
                "Zkus teď podložit, že to opravdu patří k té zákaznické schůzce, ne jen ke školení.",
            )
        },
        deep(
            "holdout-commitment-completeness",
            "Ukázka funguje lokálně. Stačí tím splnit, co jsem zákazníkovi slíbil, nebo jsme se domluvili ještě na něčem?",
        ),
    ]
}

fn history(context: Context) -> Vec<History> {
    match context {
        Context::None => Vec::new(),
        Context::Research => vec![History {
            question:"Prověř měsíční přípravu na prezentace a dema v kalendáři, poště, Teams a souborech.".into(),
            answer:"První tři schůzky jsou zpracované. První je North Star kick-off v pondělí v 9:00. Zbývá dalších pět. U jedné je neověřená připravenost dema.".into()
        }],
        Context::Verified => vec![History {
            question:"Pro koho je ukázka invoice-pilot a ze kterého repozitáře?".into(),
            answer:"Přímá e-mailová žádost i tvoje odpověď s odkazem dokládají, že je pro North Star. Ověřený repozitář je agent-demos, složka invoice-pilot. Vazba je doložená, nikoli odvozená z podobnosti názvu.".into()
        }],
        Context::Unverified => vec![History {
            question:"Co je to za demo v mém repozitáři?".into(),
            answer:"README popisuje ukázkového agenta invoice-pilot. Kalendář obsahuje podobně pojmenované školení ON AIR, ale zákazník, tvoje role ani vazba dema na konkrétní schůzku zatím nebyly ověřeny.".into()
        }],
    }
}

#[test]
fn intent_corpus_is_balanced_and_includes_the_exact_reported_regression() {
    let cases = cases();
    let holdouts = holdouts();
    for (corpus, count_per_mode) in [(&cases, 20), (&holdouts, 6)] {
        assert_eq!(corpus.len(), count_per_mode * 2);
        for mode in [Mode::Fast, Mode::Deep] {
            assert_eq!(
                corpus.iter().filter(|case| case.expected == mode).count(),
                count_per_mode
            );
        }
    }
    assert_eq!(
        cases
            .iter()
            .chain(&holdouts)
            .map(|case| case.id)
            .collect::<BTreeSet<_>>()
            .len(),
        cases.len() + holdouts.len()
    );
    let reported = cases
        .iter()
        .find(|case| case.id == "reported-demo-association")
        .unwrap();
    assert_eq!(reported.question, REPORTED_QUESTION);
    assert_eq!(reported.expected, Mode::Deep);
    assert!(history(reported.context).is_empty() && reported.meeting.is_none());
}

#[tokio::test]
#[ignore = "Requires the authorized Foundry router and a new private receipt; makes forty paced classification requests"]
async fn decision_routes_regressions_and_measures_warm_latency() {
    evaluate(
        cases(),
        "forty labeled development/regression cases, including the exact reported question; not general classifier accuracy",
    )
    .await;
}

#[tokio::test]
#[ignore = "Requires the authorized Foundry router and a separate new private receipt; makes twelve paced classification requests"]
async fn decision_routes_post_freeze_holdouts() {
    evaluate(
        holdouts(),
        "twelve balanced synthetic formulations authored after freezing the repaired rubric; not used for tuning or general classifier accuracy",
    )
    .await;
}

async fn evaluate(cases: Vec<Case>, scope: &str) {
    let root = PathBuf::from(
        std::env::var_os("TOMLOOK_LIVE_CHAT_ROOT").expect("Set TOMLOOK_LIVE_CHAT_ROOT"),
    );
    let output = PathBuf::from(
        std::env::var_os("TOMLOOK_LIVE_DECISION_OUTPUT").expect("Set TOMLOOK_LIVE_DECISION_OUTPUT"),
    );
    assert!(root.is_absolute() && output.is_absolute() && !output.exists());
    let router = Router::load(&root).await.unwrap();
    let mut outputs = Vec::new();
    let mut failure = None;
    for (index, case) in cases.iter().enumerate() {
        if index > 0 {
            tokio::time::sleep(Duration::from_millis(1200)).await;
        }
        let result = router
            .decide(case.question, &history(case.context), case.meeting)
            .await;
        match result {
            Ok(decision) => {
                if decision.mode != case.expected {
                    failure =
                        Some("A labeled routing scenario selected the wrong execution profile");
                }
                outputs.push(serde_json::json!({
                    "case_id":case.id,"question":case.question,"expected":case.expected,
                    "context":format!("{:?}",case.context),"selected_meeting":case.meeting,
                    "decision":decision
                }));
            }
            Err(error) => {
                outputs.push(
                    serde_json::json!({"case_id":case.id,"question":case.question,"expected":case.expected,"error":error}),
                );
                failure = Some("A live routing call failed");
                break;
            }
        }
    }
    let mut warm = outputs
        .iter()
        .skip(1)
        .filter_map(|value| value["decision"]["elapsed_ms"].as_u64())
        .collect::<Vec<_>>();
    warm.sort_unstable();
    let median = warm.get(warm.len() / 2).copied();
    let p95 = warm
        .get((warm.len() * 95).div_ceil(100).saturating_sub(1))
        .copied();
    let false_fast = outputs
        .iter()
        .filter(|value| value["expected"] == "deep" && value["decision"]["mode"] == "fast")
        .count();
    let false_deep = outputs
        .iter()
        .filter(|value| value["expected"] == "fast" && value["decision"]["mode"] == "deep")
        .count();
    if failure.is_none() && median.is_none_or(|latency| latency >= 3000) {
        failure = Some("Warm routing median exceeded the 3000 ms acceptance ceiling");
    }
    let receipt = serde_json::json!({
        "status":if failure.is_none() {"passed"} else {"failed"},
        "model":"Microsoft-Decision-1",
        "scope":scope,
        "expected_fast":cases.iter().filter(|case| case.expected == Mode::Fast).count(),
        "expected_deep":cases.iter().filter(|case| case.expected == Mode::Deep).count(),
        "false_fast":false_fast,"false_deep":false_deep,
        "completed_cases":outputs.len(),"warm_median_ms":median,"warm_p95_ms":p95,
        "pacing_between_requests_ms":1200,"outputs":outputs,"failure":failure
    });
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .unwrap();
    file.write_all(&serde_json::to_vec_pretty(&receipt).unwrap())
        .unwrap();
    file.sync_all().unwrap();
    assert!(failure.is_none(), "{failure:?}");
}
