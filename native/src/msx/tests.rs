use super::*;
use auth::tests::{FakeRunner, Reply};
use mcp::{CallError, ToolInfo};
use rmcp::model::CallToolResult;
use serde_json::json;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

type Handler =
    dyn Fn(Resource, &str, &Map<String, Value>) -> Result<Value, CallError> + Send + Sync;

struct Fake {
    tools: BTreeMap<Resource, Vec<ToolInfo>>,
    handler: Box<Handler>,
    calls: Mutex<Vec<(Resource, String, Value)>>,
    connects: AtomicUsize,
    refuse_connect: Mutex<Option<CallError>>,
}

struct FakeConnector(Arc<Fake>);

struct FakeSession {
    resource: Resource,
    fake: Arc<Fake>,
}

#[async_trait::async_trait]
impl mcp::Connector for FakeConnector {
    async fn connect(
        &self,
        resource: Resource,
        token: &str,
    ) -> Result<Arc<dyn mcp::Session>, CallError> {
        assert!(!token.is_empty());
        self.0.connects.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = self.0.refuse_connect.lock().unwrap().take() {
            return Err(error);
        }
        Ok(Arc::new(FakeSession {
            resource,
            fake: self.0.clone(),
        }))
    }
}

#[async_trait::async_trait]
impl mcp::Session for FakeSession {
    async fn list_tools(&self) -> Result<Vec<ToolInfo>, CallError> {
        Ok(self
            .fake
            .tools
            .get(&self.resource)
            .cloned()
            .unwrap_or_default())
    }

    async fn call(
        &self,
        name: &str,
        arguments: Map<String, Value>,
    ) -> Result<CallToolResult, CallError> {
        self.fake.calls.lock().unwrap().push((
            self.resource,
            name.to_owned(),
            Value::Object(arguments.clone()),
        ));
        (self.fake.handler)(self.resource, name, &arguments)
            .map(|value| mcp::tests::text_result(&value))
    }

    async fn close(&self) {}
}

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required})
}

fn tools() -> BTreeMap<Resource, Vec<ToolInfo>> {
    BTreeMap::from([
        (
            Resource::Dataverse,
            vec![
                ToolInfo::read(
                    "read_query",
                    schema(
                        json!({"querytext":{"type":"string"},"top":{"type":"integer"}}),
                        &["querytext"],
                    ),
                ),
                ToolInfo::read(
                    "describe",
                    schema(
                        json!({"path":{"type":"string"},"scope":{"type":"string"}}),
                        &["path"],
                    ),
                ),
                ToolInfo::read(
                    "create_record",
                    schema(json!({"tablename":{"type":"string"}}), &["tablename"]),
                ),
                ToolInfo::read("update_record", schema(json!({}), &[])),
                ToolInfo::read("delete_record", schema(json!({}), &[])),
            ],
        ),
        (
            Resource::PowerBi,
            vec![
                ToolInfo::read(
                    "ExecuteQuery",
                    schema(
                        json!({"semanticModelId":{"type":"string"},"daxQuery":{"type":"string"}}),
                        &["semanticModelId", "daxQuery"],
                    ),
                ),
                ToolInfo::read(
                    "GetSemanticModelSchema",
                    schema(
                        json!({"semanticModelId":{"type":"string"}}),
                        &["semanticModelId"],
                    ),
                ),
                ToolInfo::read("RefreshDataset", schema(json!({}), &[])),
            ],
        ),
    ])
}

const ACCOUNT: &str = "aaaaaaaa-0000-0000-0000-000000000000";
const USER: &str = "99999999-0000-0000-0000-000000000000";
const MILESTONE_A: &str = "33333333-0000-0000-0000-000000000001";
const MILESTONE_B: &str = "33333333-0000-0000-0000-000000000002";

fn describe(path: &str) -> Value {
    let columns: &[&str] = match path {
        "tables/account" => &[
            "accountid",
            "name",
            "websiteurl",
            "accountnumber",
            "ownerid",
            "msp_mstopparentid",
        ],
        "tables/opportunity" => &[
            "opportunityid",
            "name",
            "stepname",
            "estimatedclosedate",
            "estimatedvalue",
            "ownerid",
            "parentaccountid",
        ],
        "tables/msp_engagementmilestone" => &[
            "msp_engagementmilestoneid",
            "msp_name",
            "msp_milestonedate",
            "msp_milestonestatus",
            "msp_workload",
            "msp_opportunityid",
            "ownerid",
        ],
        _ => &[],
    };
    Value::String(
        std::iter::once("| Column | Type |\n|---|---|".to_owned())
            .chain(columns.iter().map(|column| format!("| {column} | text |")))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn dataverse(query: &str) -> Value {
    if query.contains("FROM systemuser") {
        assert!(
            query.contains("azureactivedirectoryobjectid = '11111111-2222-3333-4444-555555555555'"),
            "{query}"
        );
        json!({"value":[{"systemuserid": USER, "fullname": "Signed In", "internalemailaddress": "someone@example.test"}]})
    } else if query.contains("FROM contact JOIN account") {
        json!({"value":[
            {"account.accountid": ACCOUNT, "account.name": "Contoso Ltd", "account.websiteurl": "contoso.com"},
            {"account.accountid": ACCOUNT, "account.name": "Contoso Ltd", "account.websiteurl": "contoso.com"}
        ]})
    } else if query.contains("FROM account") {
        json!({"value":[
            {"accountid": ACCOUNT, "name": "Contoso Ltd", "websiteurl": "https://www.contoso.com", "msp_mstopparentid": "424242"},
            {"accountid": "bbbbbbbb-0000-0000-0000-000000000000", "name": "Contoso Pharma", "websiteurl": null}
        ]})
    } else if query.contains("FROM opportunity") {
        json!({"value":[{"opportunityid":"11111111-0000-0000-0000-000000000000","name":"Azure migration","stepname":"3-Solution","estimatedclosedate":"2026-12-15","estimatedvalue":100000}]})
    } else if query.contains("FROM msp_engagementmilestone") {
        json!({"value":[
            {"msp_engagementmilestoneid": MILESTONE_A, "msp_name":"Pilot", "msp_milestonedate":"2026-11-20", "msp_milestonestatus@OData.Community.Display.V1.FormattedValue":"On Track"},
            {"msp_engagementmilestoneid": MILESTONE_B, "msp_name":"Rollout", "msp_milestonedate":"2026-12-10"},
            {"msp_engagementmilestoneid": "33333333-0000-0000-0000-000000000003", "msp_name":"Done", "msp_milestonedate":"2026-11-25", "msp_milestonestatus@OData.Community.Display.V1.FormattedValue":"Completed"}
        ]})
    } else if query.contains("FROM task") {
        assert!(query.contains(&format!("ownerid = '{USER}'")), "{query}");
        json!({"value":[{"activityid":"44444444-0000-0000-0000-000000000000","subject":"Prepare pilot","regardingobjectid": MILESTONE_A,"regardingobjecttypecode":"msp_engagementmilestone","statecode":0}]})
    } else {
        json!({"value":[{"name":"x"}]})
    }
}

fn powerbi(name: &str, arguments: &Map<String, Value>) -> Value {
    assert_eq!(arguments["semanticModelId"], acr::MODEL_ID);
    if name == "GetSemanticModelSchema" {
        return json!({"tables":[
            {"name":"DimCustomer","columns":[{"name":"TPAccountName"},{"name":"TPID"}]},
            {"name":"DimDate","columns":[{"name":"DateID"},{"name":"FiscalYear"},{"name":"FiscalMonth"}]},
            {"name":"Measures","measures":[{"name":"$ ACR"}]}
        ]});
    }
    let dax = arguments["daxQuery"].as_str().unwrap();
    assert!(dax.contains("= \"424242\""), "{dax}");
    let today = chrono::Local::now().date_naive();
    let month = |back: u32| {
        today
            .with_day(1)
            .unwrap()
            .checked_sub_months(chrono::Months::new(back))
            .unwrap()
    };
    json!({"results":[{"tables":[{"rows":[
        {"DimDate[DateID]": month(0).format("%Y%m%d").to_string(), "[ACR]": 50.0},
        {"DimDate[DateID]": month(1).format("%Y%m%d").to_string(), "[ACR]": 1000.0},
        {"DimDate[DateID]": month(2).format("%Y%m%d").to_string(), "[ACR]": 900.0}
    ]}]}]})
}

use chrono::Datelike;

type Fixture = (
    Arc<Client>,
    Arc<Fake>,
    Arc<FakeRunner>,
    Arc<Mutex<Vec<Status>>>,
);
type Signals = (
    watch::Sender<bool>,
    watch::Receiver<bool>,
    watch::Sender<bool>,
    watch::Receiver<bool>,
);

fn fixture(handler: Box<Handler>, replies: Vec<Reply>) -> Fixture {
    let fake = Arc::new(Fake {
        tools: tools(),
        handler,
        calls: Mutex::new(Vec::new()),
        connects: AtomicUsize::new(0),
        refuse_connect: Mutex::new(None),
    });
    let runner = FakeRunner::with(replies);
    let published = Arc::new(Mutex::new(Vec::new()));
    let sink = published.clone();
    let client = Arc::new(Client::with(
        auth::tests::TENANT,
        runner.clone(),
        Arc::new(FakeConnector(fake.clone())),
        Arc::new(move |status| sink.lock().unwrap().push(status)),
    ));
    (client, fake, runner, published)
}

fn standard() -> Box<Handler> {
    Box::new(|resource, name, arguments| match resource {
        Resource::Dataverse if name == "describe" => {
            Ok(describe(arguments["path"].as_str().unwrap()))
        }
        Resource::Dataverse => Ok(dataverse(arguments["querytext"].as_str().unwrap())),
        Resource::PowerBi => Ok(powerbi(name, arguments)),
    })
}

fn signals() -> Signals {
    let (cancel, cancelled) = watch::channel(false);
    let (stop, shutdown) = watch::channel(false);
    (cancel, cancelled, stop, shutdown)
}

#[test]
fn status_summaries_follow_both_services_and_labels_are_stable() {
    let mut status = Status::new(DEFAULT_TENANT);
    assert_eq!(
        (status.phase, status.message.as_str()),
        (Phase::NotConnected, NOT_CONNECTED)
    );
    assert_eq!(status.label(), "MSX: not connected");
    status.dataverse = Service::new(Phase::Checking, "");
    status.summarize();
    assert!(status.busy() && status.label() == "MSX: checking...");
    status.dataverse = Service::new(Phase::Connected, "");
    status.powerbi = Service::new(
        Phase::Unavailable,
        "Power BI refused the connection: HTTP 403",
    );
    status.summarize();
    assert_eq!(
        (status.phase, status.message.as_str()),
        (Phase::Connected, DATAVERSE_ONLY)
    );
    status.powerbi = Service::new(Phase::Connected, "");
    status.summarize();
    assert_eq!(status.message, CONNECTED);
    status.dataverse = Service::new(Phase::SignInRequired, auth::TOKEN_TIMEOUT);
    status.powerbi = Service::new(Phase::Unavailable, "x");
    status.summarize();
    assert_eq!(
        (status.phase, status.message.as_str()),
        (Phase::SignInRequired, auth::TOKEN_TIMEOUT)
    );
    status.dataverse = Service::new(Phase::Unavailable, "Dataverse is unreachable: HTTP 503");
    status.summarize();
    assert_eq!(status.phase, Phase::Unavailable);
    status.device_code = Some("ABCD-EF12".into());
    status.summarize();
    assert!(
        status.device_code.is_none(),
        "A device code is shown only while signing in"
    );
    assert_eq!(Status::disabled().label(), "MSX: off");
    assert_eq!(Status::demo().message, DEMO);
    for phase in [
        Phase::NotConnected,
        Phase::Checking,
        Phase::SigningIn,
        Phase::Connected,
        Phase::SignInRequired,
        Phase::Unavailable,
        Phase::Disabled,
    ] {
        assert!(
            Status {
                phase,
                ..Default::default()
            }
            .label()
            .starts_with("MSX: ")
        );
        assert!(!phase.label().is_empty());
    }
    assert!(validate_tenant(DEFAULT_TENANT).is_ok());
    assert_eq!(
        validate_tenant("contoso.onmicrosoft.com").unwrap_err(),
        INVALID_TENANT
    );
}

#[tokio::test]
async fn a_quiet_check_without_a_cached_token_requires_sign_in_once_and_never_prompts() {
    let (client, fake, runner, published) = fixture(standard(), vec![Reply::Timeout]);
    let (_cancel, cancelled, _stop, shutdown) = signals();
    let failure = client
        .check(false, cancelled.clone(), shutdown.clone())
        .await
        .unwrap_err();
    assert_eq!(
        failure,
        Failure {
            message: auth::TOKEN_TIMEOUT.into(),
            retry: false
        }
    );
    let status = client.status();
    assert_eq!(status.phase, Phase::SignInRequired);
    assert_eq!(status.powerbi.phase, Phase::SignInRequired);
    assert_eq!(
        runner.calls.lock().unwrap().len(),
        1,
        "Power BI did not wait another 20 seconds"
    );
    assert!(
        runner.logins.lock().unwrap().is_empty(),
        "No interactive sign-in without a user action"
    );
    assert_eq!(fake.connects.load(Ordering::SeqCst), 0);
    client
        .check(false, cancelled.clone(), shutdown.clone())
        .await
        .unwrap_err();
    assert!(client.ai_query("SELECT name FROM account").await.is_err());
    assert_eq!(runner.calls.lock().unwrap().len(), 1, "No busy retry");
    client.check(true, cancelled, shutdown).await.unwrap();
    assert_eq!(client.status().phase, Phase::Connected);
    assert!(
        published
            .lock()
            .unwrap()
            .iter()
            .any(|status| status.phase == Phase::Checking)
    );
}

#[tokio::test]
async fn a_connected_check_allowlists_read_tools_and_resolves_the_signed_in_user() {
    let (client, fake, _runner, _published) = fixture(standard(), vec![]);
    let (_cancel, cancelled, _stop, shutdown) = signals();
    client.check(false, cancelled, shutdown).await.unwrap();
    let status = client.status();
    assert_eq!(
        (status.phase, status.message.as_str()),
        (Phase::Connected, CONNECTED)
    );
    assert_eq!(status.account.as_deref(), Some("someone@example.test"));
    assert!(status.tools.contains(&"Dataverse: read_query".to_owned()));
    assert!(status.tools.contains(&"Power BI: ExecuteQuery".to_owned()));
    assert!(
        !status
            .tools
            .iter()
            .any(|tool| tool.contains("create") || tool.contains("Refresh"))
    );
    let (discovered, allowed) = client.tool_names(Resource::Dataverse).await.unwrap();
    assert!(discovered.contains(&"delete_record".to_owned()));
    assert_eq!(allowed, ["describe", "read_query"]);
    for write in ["create_record", "update_record", "delete_record"] {
        assert!(
            client
                .call(Resource::Dataverse, write, Map::new(), false)
                .await
                .is_err()
        );
    }
    assert!(
        client
            .call(Resource::PowerBi, "RefreshDataset", Map::new(), false)
            .await
            .is_err()
    );
    assert!(
        !fake
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(_, name, _)| name.contains("record") || name.contains("Refresh")),
        "A write tool was never sent"
    );
    let user = client.whoami().await.unwrap();
    assert_eq!(
        (user.value.id.as_str(), user.value.matched_by),
        (USER, "Entra object ID")
    );
    let queries = fake
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, name, _)| name == "read_query")
        .count();
    client.whoami().await.unwrap();
    assert_eq!(
        fake.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, name, _)| name == "read_query")
            .count(),
        queries,
        "The user is resolved once per session"
    );
}

#[tokio::test]
async fn a_rejected_token_is_refreshed_once_then_sign_in_is_required_without_a_loop() {
    let (client, fake, runner, _published) =
        fixture(Box::new(|_, _, _| Err(CallError::Unauthorized)), vec![]);
    let error = client
        .ai_query("SELECT name FROM account")
        .await
        .unwrap_err();
    assert!(error.contains("sign in to MSX again"), "{error}");
    assert_eq!(runner.calls.lock().unwrap().len(), 2);
    assert_eq!(fake.connects.load(Ordering::SeqCst), 2);
    assert_eq!(client.status().dataverse.phase, Phase::SignInRequired);
    assert!(client.ai_query("SELECT name FROM account").await.is_err());
    assert_eq!(
        runner.calls.lock().unwrap().len(),
        2,
        "Later reads fail fast until the user acts"
    );
}

#[tokio::test]
async fn transient_failures_are_retryable_and_the_next_read_reconnects() {
    let failed = AtomicUsize::new(0);
    let (client, fake, _runner, _published) = fixture(
        Box::new(move |_, _, _| {
            if failed.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(CallError::Transport(
                    "HTTP 503: provider unavailable; retry later".into(),
                ))
            } else {
                Ok(json!({"value":[{"name":"Contoso"}]}))
            }
        }),
        vec![],
    );
    let error = client
        .ai_query("SELECT name FROM account")
        .await
        .unwrap_err();
    assert!(error.contains("HTTP 503"));
    assert_eq!(client.status().dataverse.phase, Phase::Unavailable);
    let value = client.ai_query("SELECT name FROM account").await.unwrap();
    assert_eq!(value["row_count"], 1);
    assert_eq!(value["untrusted_data"], true);
    assert!(
        value["query"]
            .as_str()
            .unwrap()
            .starts_with("SELECT TOP 50 ")
    );
    assert_eq!(fake.connects.load(Ordering::SeqCst), 2);
    assert_eq!(client.status().dataverse.phase, Phase::Connected);
    *fake.refuse_connect.lock().unwrap() =
        Some(CallError::Transport("cannot reach the provider".into()));
    client.disconnect().await;
    let (_cancel, cancelled, _stop, shutdown) = signals();
    let failure = client.check(true, cancelled, shutdown).await.unwrap_err();
    assert!(failure.retry, "{failure:?}");
}

#[tokio::test]
async fn accounts_are_ranked_with_evidence_and_need_confirmation() {
    let (client, fake, _runner, _published) = fixture(standard(), vec![]);
    let lookup = client
        .find_accounts(&dataverse::AccountQuery {
            domains: vec![
                "contoso.com".into(),
                "o'hara.com".into(),
                "gmail.com".into(),
            ],
            names: vec!["Contoso".into()],
        })
        .await
        .unwrap();
    assert!(lookup.requires_confirmation);
    assert_eq!(
        lookup.selection,
        dataverse::Selection::Likely {
            account_id: ACCOUNT.into()
        }
    );
    assert_eq!(lookup.candidates[0].tpid.as_deref(), Some("424242"));
    assert_eq!(lookup.rejected_inputs.len(), 2);
    assert!(
        lookup
            .sources
            .iter()
            .all(|source| source.system == "Dataverse")
    );
    let calls = fake.calls.lock().unwrap();
    let queries = calls
        .iter()
        .filter(|(_, name, _)| name == "read_query")
        .map(|(_, _, arguments)| arguments["querytext"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert!(
        queries
            .iter()
            .any(|query| query.contains("websiteurl LIKE '%contoso.com%'"))
    );
    assert!(
        queries
            .iter()
            .any(|query| query.contains("name LIKE '%Contoso%'"))
    );
    assert!(
        queries
            .iter()
            .any(|query| query.contains("contact.emailaddress1 LIKE '%@contoso.com'"))
    );
    assert!(
        queries
            .iter()
            .all(|query| !query.contains("o'hara") && !query.contains("gmail"))
    );
    assert!(calls.iter().any(|(_, name, arguments)| name == "describe" && arguments["path"] == "tables/account"));
}

#[tokio::test]
async fn account_context_reads_opportunities_future_milestones_and_owned_tasks() {
    let (client, fake, _runner, _published) = fixture(standard(), vec![]);
    let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
    let context = client
        .account_context(
            &ACCOUNT.to_uppercase(),
            today,
            today + chrono::Days::new(60),
        )
        .await
        .unwrap();
    assert_eq!(context.account_id, ACCOUNT);
    assert_eq!(context.opportunities.value[0].name, "Azure migration");
    assert_eq!(context.opportunities.source.table, "opportunity");
    let names = context
        .milestones
        .value
        .iter()
        .map(|milestone| milestone.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        ["Pilot", "Rollout"],
        "Completed milestones are excluded"
    );
    let tasks = &context.tasks;
    assert_eq!(tasks.user.as_ref().map(|user| user.id.as_str()), Some(USER));
    assert_eq!(tasks.milestones[0].has_user_task, Some(true));
    assert_eq!(tasks.milestones[1].has_user_task, Some(false));
    assert!(tasks.milestones[0].tasks[0].regarding_type_verified);
    let milestone_query = fake
        .calls
        .lock()
        .unwrap()
        .iter()
        .find_map(|(_, _, arguments)| {
            arguments["querytext"]
                .as_str()
                .filter(|query| query.contains("FROM msp_engagementmilestone"))
                .map(str::to_owned)
        })
        .unwrap();
    assert!(milestone_query.contains(&format!("opportunity.parentaccountid = '{ACCOUNT}'")));
    assert!(milestone_query.contains("msp_engagementmilestone.statecode = 0"));
    assert!(
        milestone_query.contains(">= '2026-11-01'") && milestone_query.contains("<= '2026-12-31'")
    );
    assert!(
        client
            .account_context("x' OR 1=1", today, today)
            .await
            .is_err()
    );
    assert!(
        client
            .account_context(ACCOUNT, today, today + chrono::Days::new(400))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn task_ownership_is_unknown_when_the_user_cannot_be_resolved_or_rows_are_truncated() {
    let (client, _fake, _runner, _published) = fixture(
        Box::new(|resource, name, arguments| {
            let query = arguments
                .get("querytext")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if query.contains("FROM systemuser") {
                Ok(json!({"value":[]}))
            } else if name == "describe" {
                Ok(describe(arguments["path"].as_str().unwrap()))
            } else if resource == Resource::Dataverse {
                Ok(dataverse(query))
            } else {
                Ok(json!({"value":[]}))
            }
        }),
        vec![],
    );
    let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
    let context = client
        .account_context(ACCOUNT, today, today + chrono::Days::new(60))
        .await
        .unwrap();
    assert!(context.tasks.user.is_none());
    assert!(
        context
            .tasks
            .milestones
            .iter()
            .all(|milestone| milestone.has_user_task.is_none())
    );
    assert!(
        context
            .tasks
            .note
            .as_deref()
            .unwrap()
            .starts_with("Owned-task check unavailable")
    );
}

const OTHER_OBJECT: &str = "66666666-7777-8888-9999-aaaaaaaaaaaa";
const OTHER_USER: &str = "88888888-0000-0000-0000-000000000000";
const THIRD_OBJECT: &str = "77777777-7777-8888-9999-bbbbbbbbbbbb";
const THIRD_USER: &str = "77777777-0000-0000-0000-000000000000";

/// Resolves each token identity to its own systemuser.
fn by_identity() -> Box<Handler> {
    Box::new(|resource, name, arguments| {
        let query = arguments
            .get("querytext")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if name == "describe" {
            Ok(describe(arguments["path"].as_str().unwrap()))
        } else if resource != Resource::Dataverse {
            Ok(json!({"value":[]}))
        } else if query.contains("FROM systemuser") {
            let user = [
                ("11111111-2222-3333-4444-555555555555", USER),
                (OTHER_OBJECT, OTHER_USER),
                (THIRD_OBJECT, THIRD_USER),
            ]
            .into_iter()
            .find(|(object, _)| {
                query.contains(&format!("azureactivedirectoryobjectid = '{object}'"))
            })
            .map(|(_, user)| user)
            .unwrap_or_else(|| panic!("{query}"));
            Ok(json!({"value":[{"systemuserid": user, "fullname": user}]}))
        } else if query.contains("FROM task") {
            Ok(json!({"value":[]}))
        } else {
            Ok(dataverse(query))
        }
    })
}

#[tokio::test]
async fn owned_tasks_follow_the_current_token_identity_not_a_stale_cached_user() {
    let (client, fake, runner, _published) = fixture(by_identity(), vec![]);
    let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
    let until = today + chrono::Days::new(60);
    let task_query = || {
        fake.calls
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|(_, _, arguments)| {
                arguments["querytext"]
                    .as_str()
                    .filter(|query| query.contains("FROM task"))
                    .map(str::to_owned)
            })
            .unwrap()
    };
    let context = client.account_context(ACCOUNT, today, until).await.unwrap();
    assert_eq!(
        context.tasks.user.as_ref().map(|user| user.id.as_str()),
        Some(USER)
    );
    assert!(task_query().contains(&format!("ownerid = '{USER}'")));

    // The Azure CLI now issues another account's token, e.g. a refresh after `az login`.
    *runner.identity.lock().unwrap() = Some(("other@example.test", OTHER_OBJECT));
    client.tokens.forget(Resource::Dataverse).await;
    client.drop_session(Resource::Dataverse).await;
    let context = client.account_context(ACCOUNT, today, until).await.unwrap();
    assert_eq!(
        context.tasks.user.as_ref().map(|user| user.id.as_str()),
        Some(OTHER_USER)
    );
    let query = task_query();
    assert!(
        query.contains(&format!("ownerid = '{OTHER_USER}'")) && !query.contains(USER),
        "{query}"
    );

    // An explicit sign-in as a third account resolves that account's user.
    *runner.identity.lock().unwrap() = Some(("third@example.test", THIRD_OBJECT));
    let (_cancel, cancelled, _stop, shutdown) = signals();
    client.sign_in(false, cancelled, shutdown).await.unwrap();
    assert_eq!(
        client.status().account.as_deref(),
        Some("third@example.test")
    );
    let context = client.account_context(ACCOUNT, today, until).await.unwrap();
    assert_eq!(
        context.tasks.user.as_ref().map(|user| user.id.as_str()),
        Some(THIRD_USER)
    );
    assert!(task_query().contains(&format!("ownerid = '{THIRD_USER}'")));
}

#[tokio::test]
async fn acr_is_labelled_with_model_unit_and_freshness_or_explicitly_unavailable() {
    let (client, _fake, _runner, _published) = fixture(standard(), vec![]);
    let customer = acr::Customer::new(Some("424242"), Some("Contoso Ltd")).unwrap();
    let today = chrono::Local::now().date_naive();
    let acr::Acr::Available(series) = client.acr(&customer, 6, today).await else {
        panic!("ACR should be available");
    };
    assert_eq!(
        (series.model_name, series.unit, series.matched_by),
        (acr::MODEL_NAME, "USD", "TPID")
    );
    let kinds = series
        .months
        .iter()
        .map(|month| month.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            acr::Kind::Actual,
            acr::Kind::Actual,
            acr::Kind::PartialActual
        ]
    );
    assert_eq!(series.latest_month, Some(today.with_day(1).unwrap()));
    assert!(series.prediction_note.starts_with("Prediction omitted"));
    assert!(matches!(
        client.acr(&customer, 3, today).await,
        acr::Acr::Unavailable { .. }
    ));

    let (missing, _fake, _runner, _published) = fixture(
        Box::new(|resource, name, _| match (resource, name) {
            (Resource::PowerBi, "GetSemanticModelSchema") => {
                Ok(json!({"tables":[{"name":"Other"}]}))
            }
            _ => Ok(json!({"value":[]})),
        }),
        vec![],
    );
    let acr::Acr::Unavailable { reason } = missing.acr(&customer, 6, today).await else {
        panic!("A model without the ACR measure is unavailable");
    };
    assert!(reason.contains("$ ACR"));
    let (signed_out, _fake, _runner, _published) = fixture(standard(), vec![Reply::Timeout]);
    assert_eq!(
        signed_out.acr(&customer, 6, today).await,
        acr::Acr::Unavailable {
            reason: auth::TOKEN_TIMEOUT.into()
        }
    );
}

#[tokio::test]
async fn explicit_sign_in_shows_a_device_code_then_connects_and_disconnect_forgets_tokens() {
    let (client, fake, runner, published) = fixture(standard(), vec![]);
    let (_cancel, cancelled, _stop, shutdown) = signals();
    client.sign_in(true, cancelled, shutdown).await.unwrap();
    assert_eq!(runner.logins.lock().unwrap().len(), 1);
    assert!(
        published
            .lock()
            .unwrap()
            .iter()
            .any(|status| status.phase == Phase::SigningIn
                && status.device_code.as_deref() == Some("ABCD-EF12")
                && status.message == DEVICE_PROMPT)
    );
    assert_eq!(client.status().phase, Phase::Connected);
    let tokens = runner.calls.lock().unwrap().len();
    client.disconnect().await;
    let status = client.status();
    assert_eq!(
        (status.phase, status.message.as_str()),
        (Phase::NotConnected, DISCONNECTED)
    );
    assert!(status.account.is_none() && status.tools.is_empty());
    assert_eq!(
        client
            .ai_query("SELECT name FROM account")
            .await
            .unwrap_err(),
        DISCONNECTED,
        "Nothing reconnects silently after Disconnect"
    );
    assert_eq!(runner.calls.lock().unwrap().len(), tokens);
    let (_cancel, cancelled, _stop, shutdown) = signals();
    client.check(true, cancelled, shutdown).await.unwrap();
    client.ai_query("SELECT name FROM account").await.unwrap();
    assert_eq!(
        runner.calls.lock().unwrap().len(),
        tokens + 2,
        "Disconnect forgot both in-memory tokens"
    );
    assert!(fake.connects.load(Ordering::SeqCst) >= 4);
}

#[tokio::test]
async fn a_cancelled_check_reports_cancellation_and_leaves_no_busy_state() {
    let (client, _fake, _runner, _published) = fixture(standard(), vec![]);
    let (_cancel, cancelled) = watch::channel(true);
    let (_stop, shutdown) = watch::channel(false);
    let failure = client.check(false, cancelled, shutdown).await.unwrap_err();
    assert_eq!(failure.message, CHECK_CANCELLED);
    assert!(!client.status().busy());
}

#[tokio::test]
async fn copilot_tools_appear_only_when_connected_and_spend_the_shared_read_budget() {
    use github_copilot_sdk::tool::ToolHandler;
    let (client, fake, _runner, _published) = fixture(standard(), vec![]);
    let recorder = Arc::new(crate::evidence::Recorder::new(Arc::new(
        crate::evidence::Redactor::default(),
    )));
    let remaining = Arc::new(AtomicUsize::new(1));
    let mut names = std::collections::BTreeSet::new();
    let mut registered = Vec::new();
    assert!(
        !tools::register(&client, &mut names, &mut registered, &recorder, &remaining),
        "MSX tools are unavailable before MSX is connected"
    );
    let (_cancel, cancelled, _stop, shutdown) = signals();
    client.check(false, cancelled, shutdown).await.unwrap();
    assert!(tools::register(
        &client,
        &mut names,
        &mut registered,
        &recorder,
        &remaining
    ));
    assert_eq!(names.len(), 3);
    assert_eq!(registered.len(), 3);
    let handler = tools::Read {
        name: tools::QUERY_TOOL,
        client: client.clone(),
        recorder,
        remaining,
    };
    let invoke = |arguments: Value| {
        serde_json::from_value::<github_copilot_sdk::ToolInvocation>(json!({
            "sessionId": "fixture-session",
            "toolCallId": "fixture-call",
            "toolName": tools::QUERY_TOOL,
            "arguments": arguments,
        }))
        .unwrap()
    };
    let result = serde_json::to_value(
        handler
            .call(invoke(json!({"query":"SELECT name FROM account"})))
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(result["resultType"], "success");
    let text: Value = serde_json::from_str(result["textResultForLlm"].as_str().unwrap()).unwrap();
    assert_eq!(text["untrusted_data"], true);
    assert_eq!(text["tables"], "account");
    let sent = fake.calls.lock().unwrap().len();
    for arguments in [
        json!({"query":"SELECT name FROM account"}),
        json!({"query":"DELETE FROM account"}),
        json!({"query":"SELECT name FROM account", "table": "task"}),
    ] {
        let result = serde_json::to_value(handler.call(invoke(arguments)).await.unwrap()).unwrap();
        assert_eq!(result["resultType"], "failure");
    }
    assert_eq!(
        fake.calls.lock().unwrap().len(),
        sent,
        "Exhausted budgets and invalid queries send nothing"
    );
}
