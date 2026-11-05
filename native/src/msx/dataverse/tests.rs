use super::*;

#[test]
fn sql_literals_escape_quotes_and_like_wildcards() {
    assert_eq!(sql_string("O'Brien"), "'O''Brien'");
    assert_eq!(sql_string("x' OR '1'='1"), "'x'' OR ''1''=''1'");
    assert_eq!(like_contains("50%_[a]"), "'%50[%][_][[]a]%'");
    assert_eq!(like_contains("Müller's"), "'%Müller''s%'");
}

#[test]
fn domains_are_normalized_ascii_and_reject_injection_or_non_domains() {
    assert_eq!(domain("Contoso.COM").unwrap(), "contoso.com");
    assert_eq!(domain(" @www.Fabrikam.cz. ").unwrap(), "fabrikam.cz");
    let unicode = domain("příklad.cz").unwrap();
    assert!(unicode.starts_with("xn--") && unicode.ends_with(".cz") && unicode.is_ascii());
    for invalid in [
        "o'brien.com",
        "evil.com' OR 1=1 --",
        "contoso.com;drop",
        "con%toso.com",
        "con_toso.com",
        "localhost",
        "1.2.3.4",
        "[::1]",
        "-bad.com",
        "a..b.com",
        "contoso.123",
        "",
        "contoso.com/path",
        "user@contoso.com",
    ] {
        assert!(domain(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn inputs_are_bounded_deduplicated_and_public_mail_is_not_a_customer() {
    let query = AccountQuery {
        domains: vec![
            "contoso.com".into(),
            "CONTOSO.com".into(),
            "gmail.com".into(),
            "bad'domain.com".into(),
            "a.cz".into(),
            "b.cz".into(),
            "c.cz".into(),
            "d.cz".into(),
            "e.cz".into(),
        ],
        names: vec![
            "Contoso a.s.".into(),
            "<script>".into(),
            "Contoso a.s.".into(),
        ],
    };
    let (domains, names, rejected) = query.prepare();
    assert_eq!(domains, ["contoso.com", "a.cz", "b.cz", "c.cz", "d.cz"]);
    assert_eq!(names, ["Contoso a.s."]);
    assert!(
        rejected
            .iter()
            .any(|reason| reason.starts_with("gmail.com: public"))
    );
    assert!(
        rejected
            .iter()
            .any(|reason| reason.contains("bad'domain.com"))
    );
    assert!(rejected.iter().any(|reason| reason.contains(INVALID_NAME)));
    assert!(company_name("Ž").is_err());
    assert_eq!(company_name("  Škoda   Auto ").unwrap(), "Škoda Auto");
}

#[test]
fn name_similarity_ignores_case_diacritics_and_legal_forms() {
    assert_eq!(similarity("ŠKODA AUTO a.s.", "Skoda Auto"), 1.0);
    assert_eq!(similarity("Contoso, Ltd.", "contoso"), 1.0);
    assert!(similarity("Contoso Pharma", "Contoso") >= 0.75);
    assert!(similarity("Fabrikam", "Contoso") < 0.1);
    assert_eq!(similarity("a.s.", "Contoso"), 0.0);
}

fn candidate(id: char, name: &str, website: Option<&str>) -> AccountCandidate {
    let id = format!("{}-0000-0000-0000-000000000000", id.to_string().repeat(8));
    account(&json!({
        "accountid": id,
        "name": name,
        "websiteurl": website,
        "msp_mstopparentid": "123456",
        "ownerid": "00000000-0000-0000-0000-00000000000a",
        "owneridname": "Owner Name"
    }))
    .unwrap()
}

#[test]
fn candidates_carry_evidence_and_ambiguity_is_never_silently_resolved() {
    let domains = vec!["contoso.com".to_owned()];
    let names = vec!["Contoso".to_owned()];
    let mut strong = candidate('a', "Contoso Ltd", Some("https://www.contoso.com/cz"));
    score(
        &mut strong,
        &domains,
        &names,
        &BTreeMap::from([("contoso.com".to_owned(), 4)]),
    );
    assert!(
        strong.confidence >= 0.9 && strong.confidence < 1.0,
        "{}",
        strong.confidence
    );
    assert_eq!(strong.tpid.as_deref(), Some("123456"));
    assert_eq!(
        strong
            .owner
            .as_ref()
            .and_then(|owner| owner.name.as_deref()),
        Some("Owner Name")
    );
    assert!(
        strong
            .url
            .starts_with("https://microsoftsales.crm.dynamics.com/main.aspx?etn=account&id=")
    );
    assert!(matches!(strong.evidence[0], Evidence::WebsiteDomain { .. }));
    assert!(
        strong
            .evidence
            .iter()
            .any(|item| matches!(item, Evidence::ContactDomain { contacts: 4, .. }))
    );
    let mut weak = candidate('b', "Contoso Pharma", None);
    score(&mut weak, &domains, &names, &BTreeMap::new());
    assert!(weak.confidence < 0.6);
    let mut subsidiary = candidate('c', "Contoso Services", Some("services.contoso.com"));
    score(&mut subsidiary, &domains, &names, &BTreeMap::new());
    assert!(
        subsidiary
            .evidence
            .iter()
            .any(|item| matches!(item, Evidence::WebsiteDomain { .. }))
    );
    assert_eq!(
        select(&[strong.clone(), weak.clone()]),
        Selection::Likely {
            account_id: strong.id.clone()
        }
    );
    let mut ranked = vec![subsidiary.clone(), strong.clone()];
    ranked.sort_by(|left, right| right.confidence.total_cmp(&left.confidence));
    assert_eq!(
        select(&ranked),
        Selection::Ambiguous,
        "Two close website matches need the user"
    );
    assert_eq!(select(&[weak]), Selection::Ambiguous);
    assert_eq!(select(&[]), Selection::NoMatch);
}

#[test]
fn a_subsidiary_outside_the_territory_is_represented_by_its_territory_top_parent() {
    let with = |id: char, name: &str, tpid: Option<&str>, confidence: f32| {
        let mut candidate = candidate(id, name, None);
        candidate.tpid = tpid.map(str::to_owned);
        candidate.confidence = confidence;
        candidate.evidence = vec![Evidence::ContactDomain {
            domain: "drmax.eu".into(),
            contacts: 2,
        }];
        candidate
    };
    let penta = with('a', "PENTA INVESTMENTS", Some("9582911"), 0.0);
    let territory = vec![penta.clone(), with('e', "ALZA.CZ", Some("11202098"), 0.0)];
    let candidates = vec![
        with('b', "DR. MAX BDC", Some("9582911"), 0.65),
        with('c', "DR. MAX", Some("45476255"), 0.75),
        with('a', "PENTA INVESTMENTS", Some("9582911"), 0.55),
    ];
    let mut ranked = apply_territory(candidates, &territory);
    ranked.sort_by(|left, right| right.confidence.total_cmp(&left.confidence));
    assert_eq!(ranked.len(), 2, "The subsidiary merges into its territory parent");
    assert_eq!((ranked[0].name.as_str(), ranked[0].confidence), ("PENTA INVESTMENTS", 0.95));
    assert!(ranked[0].evidence.contains(&Evidence::Territory));
    assert!(ranked[0].evidence.contains(&Evidence::TopParent {
        account: "DR. MAX BDC".into(),
        tpid: "9582911".into()
    }));
    assert_eq!(
        select(&ranked),
        Selection::Likely {
            account_id: penta.id.clone()
        }
    );
    let close = vec![
        with('a', "PENTA INVESTMENTS", Some("9582911"), 0.4),
        with('c', "DR. MAX", Some("45476255"), 0.65),
    ];
    let mut close = apply_territory(close, &territory);
    close.sort_by(|left, right| right.confidence.total_cmp(&left.confidence));
    assert_eq!(
        select(&close),
        Selection::Likely {
            account_id: penta.id.clone()
        },
        "One territory account ahead of outside accounts is the expected customer"
    );
    let unchanged = apply_territory(vec![with('c', "DR. MAX", Some("45476255"), 0.75)], &[]);
    assert_eq!(unchanged[0].confidence, 0.75);
    assert!(!unchanged[0].evidence.contains(&Evidence::Territory));
}

#[test]
fn records_parse_joined_columns_formatted_values_and_dates() {
    let opportunity = opportunity(&json!({
        "opportunityid": "{11111111-1111-1111-1111-111111111111}",
        "name": "Azure migration",
        "stepname": "3-Solution",
        "estimatedclosedate": "2026-12-15T00:00:00Z",
        "estimatedvalue": "125000.5",
        "ownerid": "22222222-2222-2222-2222-222222222222"
    }))
    .unwrap();
    assert_eq!(opportunity.id, "11111111-1111-1111-1111-111111111111");
    assert_eq!(opportunity.stage.as_deref(), Some("3-Solution"));
    assert_eq!(
        opportunity.estimated_close,
        NaiveDate::from_ymd_opt(2026, 12, 15)
    );
    assert_eq!(opportunity.estimated_value, Some(125000.5));
    let milestone = milestone(&json!({
        "msp_engagementmilestone.msp_engagementmilestoneid": "33333333-3333-3333-3333-333333333333",
        "msp_engagementmilestone.msp_name": "Pilot go-live",
        "msp_engagementmilestone.msp_milestonedate": "11/20/2026 12:00:00 AM",
        "msp_engagementmilestone.msp_milestonestatus": 861980000,
        "msp_milestonestatus@OData.Community.Display.V1.FormattedValue": "On Track",
        "msp_engagementmilestone.msp_workload": "Azure AI",
        "msp_engagementmilestone.msp_opportunityid": "11111111-1111-1111-1111-111111111111"
    }))
    .unwrap();
    assert_eq!(milestone.due, NaiveDate::from_ymd_opt(2026, 11, 20));
    assert_eq!(milestone.status.as_deref(), Some("On Track"));
    assert_eq!(milestone.status_code, Some(861980000));
    assert!(open_milestone(&milestone));
    let mut closed = milestone.clone();
    closed.status = Some("Completed".into());
    assert!(!open_milestone(&closed));
    closed.status = None;
    assert!(
        open_milestone(&closed),
        "An unknown label is not treated as closed"
    );
    assert!(super::milestone(&json!({"msp_name":"No id"})).is_none());
}

#[test]
fn owned_tasks_require_the_milestone_id_and_type() {
    let milestones = BTreeSet::from(["33333333-3333-3333-3333-333333333333".to_owned()]);
    let row = |kind: Option<&str>, regarding: &str| {
        json!({
            "activityid": "44444444-4444-4444-4444-444444444444",
            "subject": "Prepare architecture review",
            "scheduledend": "2026-11-18",
            "statecode": 0,
            "regardingobjectid": regarding,
            "regardingobjecttypecode": kind
        })
    };
    let task = owned_task(
        &row(
            Some("msp_engagementmilestone"),
            "33333333-3333-3333-3333-333333333333",
        ),
        &milestones,
    )
    .unwrap();
    assert!(task.regarding_type_verified);
    assert_eq!(task.due, NaiveDate::from_ymd_opt(2026, 11, 18));
    assert!(
        owned_task(
            &row(Some("opportunity"), "33333333-3333-3333-3333-333333333333"),
            &milestones
        )
        .is_none()
    );
    assert!(
        owned_task(
            &row(None, "55555555-5555-5555-5555-555555555555"),
            &milestones
        )
        .is_none()
    );
    assert!(
        !owned_task(
            &row(None, "33333333-3333-3333-3333-333333333333"),
            &milestones
        )
        .unwrap()
        .regarding_type_verified
    );
}

#[test]
fn schema_discovery_reads_markdown_and_json_and_selects_only_known_optional_columns() {
    let markdown = Value::String(
        "# account\n| Column | Type |\n|---|---|\n| accountid | uniqueidentifier |\n| name | nvarchar |\n| `msp_mstopparentid` | nvarchar |\n- websiteurl (nvarchar)\nExample: `SELECT name FROM account`".into(),
    );
    let schema = TableSchema::parse(&markdown);
    assert!(schema.known());
    for column in ["accountid", "name", "msp_mstopparentid", "websiteurl"] {
        assert!(schema.has(column), "{column}");
    }
    assert!(!schema.has("Column") && !schema.has("SELECT name FROM account"));
    assert_eq!(
        schema.select(
            &["accountid", "name"],
            &["websiteurl", "accountnumber"],
            &["msp_mstopparentid", "msp_tpid"]
        ),
        ["accountid", "name", "websiteurl", "msp_mstopparentid"]
    );
    let json_schema = TableSchema::parse(
        &json!({"columns":[{"LogicalName":"opportunityid"},{"logicalName":"name"},{"logicalName":"stepname"}]}),
    );
    assert!(json_schema.known() && json_schema.has("stepname"));
    let unknown = TableSchema::default();
    assert_eq!(
        unknown.select(&["accountid"], &["websiteurl"], &["msp_tpid"]),
        ["accountid", "websiteurl"],
        "Without discovery only standard columns are used"
    );
}

#[test]
fn ai_queries_are_single_selects_on_allowlisted_tables_with_a_row_bound() {
    let (sql, tables) =
        validate_ai_query("select name, websiteurl from account where name like '%Contoso%'")
            .unwrap();
    assert_eq!(
        sql,
        "SELECT TOP 50 name, websiteurl from account where name like '%Contoso%'"
    );
    assert_eq!(tables, ["account"]);
    let (sql, tables) = validate_ai_query(
        "SELECT TOP 10 m.msp_name FROM msp_engagementmilestone m JOIN opportunity o ON m.msp_opportunityid = o.opportunityid WHERE o.name = 'Delete; drop -- it'",
    )
    .unwrap();
    assert!(sql.starts_with("SELECT TOP 10 m.msp_name"));
    assert_eq!(tables, ["msp_engagementmilestone", "opportunity"]);
    for invalid in [
        "DELETE FROM account",
        "SELECT name FROM account; DELETE FROM account",
        "SELECT name FROM account -- comment",
        "SELECT name FROM account /* x */",
        "SELECT name INTO x FROM account",
        "SELECT name FROM salesorder",
        "SELECT name FROM account, contact",
        "SELECT name FROM account a, contact c",
        "SELECT TOP 0 name FROM account",
        "SELECT TOP 10 PERCENT name FROM account",
        "SELECT TOP x name FROM account",
        "SELECT [name] FROM account",
        "SELECT \"name\" FROM account",
        "SELECT name FROM account WHERE name = 'unterminated",
        "SELECT name FROM account UNION SELECT fullname FROM systemuser",
        "SELECT name FROM account WHERE @x = 1",
        "SELECT 1",
        "WITH x AS (SELECT name FROM account) SELECT name FROM x",
        "SELECT name FROM account WHERE name = N'x' EXEC xp_cmdshell",
    ] {
        assert!(validate_ai_query(invalid).is_err(), "{invalid}");
    }
    assert!(
        validate_ai_query(&format!(
            "SELECT name FROM account WHERE name = '{}'",
            "x".repeat(4000)
        ))
        .is_err()
    );
}

#[test]
fn ai_queries_check_every_table_source_and_reject_comma_joins_aliases_and_set_operators() {
    let (_, tables) = validate_ai_query(
        "SELECT a.name, c.fullname FROM account AS a\nLEFT OUTER JOIN contact AS c ON a.accountid = c.parentcustomerid AND LEFT(c.fullname, 1) IN ('A', 'B') CROSS JOIN systemuser WHERE a.name LIKE 'C%'",
    )
    .unwrap();
    assert_eq!(tables, ["account", "contact", "systemuser"]);
    for bypass in [
        "SELECT s.name FROM account AS a, salesorder s",
        "SELECT s.name FROM account a JOIN contact c ON a.accountid = c.parentcustomerid, salesorder s",
        "SELECT s.name FROM account AS a,\nsalesorder s",
        "SELECT s.name FROM account a\n\t,\n salesorder s WHERE 1 = 1",
        "SELECT s.name FROM account a JOIN contact c ON (a.accountid = c.parentcustomerid), salesorder s",
        "SELECT s.name FROM account AS a, contact c",
        "SELECT name FROM account AS",
        "SELECT name FROM account a JOIN contact c",
        "SELECT name FROM account a JOIN salesorder s ON a.accountid = s.customerid",
        "SELECT name FROM account a INNER JOIN contact c ON a.accountid = c.parentcustomerid JOIN salesorder s ON 1 = 1",
        "SELECT name FROM account a CROSS JOIN salesorder",
        "SELECT name FROM account /* x */, salesorder",
        "SELECT name FROM account -- x\n, salesorder",
        "SELECT name FROM [salesorder]",
        "SELECT name FROM account a JOIN [salesorder] s ON 1 = 1",
        "SELECT name FROM \"salesorder\"",
        "SELECT name FROM `salesorder`",
        "SELECT name FROM dbo.salesorder",
        "SELECT name FROM dbo.account",
        "SELECT name FROM account.salesorder",
        "SELECT name FROM msx..account",
        "SELECT a.name FROM account a CROSS APPLY salesorder s",
        "SELECT a.name FROM account a OUTER APPLY (SELECT TOP 1 name FROM salesorder) s",
        "SELECT name FROM account UNION ALL SELECT name FROM salesorder",
        "SELECT name FROM account EXCEPT SELECT name FROM contact",
        "SELECT name FROM account; SELECT name FROM salesorder",
        "SELECT name FROM account SELECT name FROM salesorder",
        "SELECT name FROM account WHERE accountid IN (SELECT customerid FROM salesorder)",
        "SELECT (SELECT TOP 1 name FROM salesorder) FROM account",
        "SELECT name FROM (SELECT name FROM salesorder) AS x",
        "SELECT name FROM account WHERE 1 = 1 JOIN salesorder s ON 1 = 1",
        "SELECT name FROM account a JOIN contact c ON (a.accountid = c.parentcustomerid",
        "SELECT TRIM('x' FROM name) FROM account",
        "SELECT name FROM account, account",
    ] {
        assert!(validate_ai_query(bypass).is_err(), "{bypass}");
    }
}

#[test]
fn ai_query_row_bound_keeps_distinct_all_and_existing_top_valid() {
    let limited = |sql: &str| validate_ai_query(sql).unwrap().0;
    assert_eq!(
        limited("SELECT DISTINCT name FROM account"),
        "SELECT DISTINCT TOP 50 name FROM account"
    );
    assert_eq!(
        limited("select distinct\n name FROM account"),
        "SELECT DISTINCT TOP 50 name FROM account"
    );
    assert_eq!(
        limited("SELECT DISTINCT TOP 5 name FROM account"),
        "SELECT DISTINCT TOP 5 name FROM account"
    );
    assert_eq!(
        limited("SELECT TOP 500 name FROM account"),
        "SELECT TOP 50 name FROM account"
    );
    assert_eq!(
        limited("SELECT DISTINCT TOP 500 name FROM account"),
        "SELECT DISTINCT TOP 50 name FROM account"
    );
    assert_eq!(
        limited("SELECT ALL name FROM account"),
        "SELECT ALL TOP 50 name FROM account"
    );
    assert_eq!(
        limited("SELECT TOP (10) name FROM account"),
        "SELECT TOP (10) name FROM account"
    );
    assert_eq!(
        limited("SELECT TOP(500) name FROM account"),
        "SELECT TOP(50) name FROM account"
    );
    assert_eq!(
        limited("SELECT COUNT(DISTINCT name) FROM account"),
        "SELECT TOP 50 COUNT(DISTINCT name) FROM account"
    );
    for invalid in [
        "SELECT DISTINCT TOP x name FROM account",
        "SELECT TOP (x) name FROM account",
        "SELECT TOP (0) name FROM account",
        "SELECT TOP 5 PERCENT name FROM account",
        "SELECT TOP 5 WITH TIES name FROM account ORDER BY name",
    ] {
        assert!(validate_ai_query(invalid).is_err(), "{invalid}");
    }
}
