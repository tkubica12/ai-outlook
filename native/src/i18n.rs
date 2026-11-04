use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    En,
    Cs,
}

impl Language {
    pub fn code(self) -> &'static str {
        match self {
            Self::En => "EN",
            Self::Cs => "CZ",
        }
    }

    pub fn other(self) -> Self {
        match self {
            Self::En => Self::Cs,
            Self::Cs => Self::En,
        }
    }

    pub fn text<'a>(self, text: &'a str) -> Cow<'a, str> {
        if self == Self::En || text.is_empty() {
            return Cow::Borrowed(text);
        }
        for &(en, cs) in TEXTS {
            if text == en {
                return Cow::Borrowed(cs);
            }
        }
        if text.contains('\n') {
            return Cow::Owned(
                text.lines()
                    .map(|line| self.text(line))
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        for &(en, cs) in PATTERNS {
            if let Some(values) = capture(en, text) {
                let mut translated = String::with_capacity(text.len());
                let mut remaining = cs;
                while let Some(start) = remaining.find('{') {
                    translated.push_str(&remaining[..start]);
                    let (index, tail) = remaining[start + 1..]
                        .split_once('}')
                        .expect("Catalog has balanced placeholders");
                    let index: usize = index.parse().expect("Catalog placeholders are numbered");
                    translated.push_str(&self.text(values[index]));
                    remaining = tail;
                }
                translated.push_str(remaining);
                return Cow::Owned(translated);
            }
        }
        Cow::Borrowed(text)
    }

    pub fn month(self, date: NaiveDate) -> String {
        const MONTHS: [&str; 12] = [
            "leden",
            "únor",
            "březen",
            "duben",
            "květen",
            "červen",
            "červenec",
            "srpen",
            "září",
            "říjen",
            "listopad",
            "prosinec",
        ];
        match self {
            Self::En => date.format("%B %Y").to_string(),
            Self::Cs => format!("{} {}", MONTHS[date.month0() as usize], date.year()),
        }
    }

    pub fn date(self, date: NaiveDate, full: bool) -> String {
        const DAYS: [&str; 7] = ["Po", "Út", "St", "Čt", "Pá", "So", "Ne"];
        const FULL: [&str; 7] = [
            "pondělí",
            "úterý",
            "středa",
            "čtvrtek",
            "pátek",
            "sobota",
            "neděle",
        ];
        match self {
            Self::En => date
                .format(if full { "%A, %e %B %Y" } else { "%a %e" })
                .to_string(),
            Self::Cs if full => format!(
                "{}, {}. {}. {}",
                FULL[date.weekday().num_days_from_monday() as usize],
                date.day(),
                date.month(),
                date.year()
            ),
            Self::Cs => format!(
                "{} {}.",
                DAYS[date.weekday().num_days_from_monday() as usize],
                date.day()
            ),
        }
    }

    pub fn range(self, first: NaiveDate, last: NaiveDate) -> String {
        match self {
            Self::En => format!("{} - {}", first.format("%e %b"), last.format("%e %b %Y")),
            Self::Cs => format!(
                "{}. {}. – {}. {}. {}",
                first.day(),
                first.month(),
                last.day(),
                last.month(),
                last.year()
            ),
        }
    }

    pub fn insight(self, value: &serde_json::Value) -> Option<&serde_json::Value> {
        if self == Self::En {
            Some(value)
        } else {
            value.get("cs").filter(|value| value.is_object())
        }
    }
}

fn capture<'a>(pattern: &str, text: &'a str) -> Option<Vec<&'a str>> {
    let mut parts = pattern.split("{}").peekable();
    let mut remaining = text.strip_prefix(parts.next()?)?;
    let mut values = Vec::new();
    while let Some(suffix) = parts.next() {
        if parts.peek().is_none() {
            // The last value may itself contain the closing text, e.g. dots in "31.10.2026.".
            values.push(remaining.strip_suffix(suffix)?);
            remaining = "";
        } else {
            let end = remaining.find(suffix)?;
            values.push(&remaining[..end]);
            remaining = &remaining[end + suffix.len()..];
        }
    }
    remaining.is_empty().then_some(values)
}

pub const TEXTS: &[(&str, &str)] = &[
    ("Invitations", "Pozvánky"),
    ("Previous invitations", "Předchozí pozvánky"),
    ("Next invitations", "Další pozvánky"),
    ("Important", "Důležité"),
    ("Snoozed", "Odložené"),
    ("My team", "Můj tým"),
    ("Unanswered invitations, prioritized without AI", "Nezodpovězené pozvánky, seřazené bez AI"),
    ("Invitation context: calendar filters are temporarily ignored.", "Kontext pozvánky: filtry kalendáře jsou dočasně vypnuté."),
    ("Selected invitation", "Vybraná pozvánka"),
    ("Selected invitations", "Vybrané pozvánky"),
    ("Select invitation", "Označit pozvánku"),
    ("Select page", "Označit stránku"),
    ("Clear selection", "Zrušit výběr"),
    ("Selected invitation actions", "Akce pro vybrané pozvánky"),
    ("Decline without sending", "Odmítnout bez odeslání"),
    ("Follow selected invitations", "Sledovat vybrané pozvánky"),
    ("Declines every listed invitation without notifying the organizer.", "Odmítne všechny uvedené pozvánky bez upozornění organizátora."),
    ("Confirm for all selected invitations", "Potvrdit pro všechny vybrané pozvánky"),
    ("Review individual progress and outcomes in Messages. Failed or uncertain changes are not retried automatically.", "Průběh a výsledky jednotlivých změn najdete ve Zprávách. Neúspěšné nebo nejisté změny se automaticky neopakují."),
    ("Selected invitations changed; close this dialog and select them again", "Vybrané pozvánky se změnily; zavřete tento dialog a označte je znovu"),
    ("Select between 1 and 2,000 invitations", "Označte 1 až 2 000 pozvánek"),
    ("An invitation was selected more than once", "Jedna pozvánka byla označena vícekrát"),
    ("Invitation actions could not be queued; nothing was submitted", "Akce pro pozvánky se nepodařilo zařadit do fronty; nic nebylo odesláno"),
    ("Day + neighbors", "Den a okolí"),
    ("Whole week", "Celý týden"),
    ("Recurring invitation: responses apply only to this occurrence.", "Opakovaná pozvánka: odpověď platí pouze pro tento výskyt."),
    ("Checking unanswered invitations...", "Ověřuji nezodpovězené pozvánky..."),
    ("Last checked:", "Poslední kontrola:"),
    ("Cached invitations; a complete queue refresh has not finished yet.", "Uložené pozvánky; úplné načtení fronty zatím neproběhlo."),
    ("Offline: showing saved invitations.", "Offline: zobrazuji uložené pozvánky."),
    ("Next six months. Reading is not accepting; snoozing changes only Tomlook.", "Příštích šest měsíců. Přečtení není přijetí; odložení mění pouze Tomlook."),
    ("Customer meeting", "Schůzka se zákazníkem"),
    ("Highlighted organizer", "Zvýrazněný organizátor"),
    ("Organizer from my team", "Organizátor z mého týmu"),
    ("External organizer - customer not confirmed", "Externí organizátor - zákazník nepotvrzen"),
    ("External participant - customer not confirmed", "Externí účastník - zákazník nepotvrzen"),
    ("Quiet by your choice", "Ztišeno podle vaší volby"),
    ("Internal broadcast or optional event", "Hromadná nebo nepovinná akce"),
    ("Other internal invitation", "Jiná interní pozvánka"),
    ("Conflicts not checked", "Konflikty zatím neověřeny"),
    ("No conflicts in recently refreshed calendar", "Bez konfliktů v čerstvě načteném kalendáři"),
    ("Calendar conflict - review before responding", "Konflikt v kalendáři - zkontrolujte před odpovědí"),
    ("Invitation changed", "Pozvánka se změnila"),
    ("Newly seen", "Nově nalezeno"),
    ("Snoozed until:", "Odloženo do:"),
    ("No invitations in this view.", "V tomto přehledu nejsou žádné pozvánky."),
    ("Open in calendar", "Otevřít v kalendáři"),
    ("Unsnooze", "Vrátit z odložených"),
    ("Snooze 1 hour", "Odložit o hodinu"),
    ("Snooze 1 day", "Odložit o den"),
    ("Snooze 1 week", "Odložit o týden"),
    ("Highlight this organizer", "Zvýraznit tohoto organizátora"),
    ("Quiet this organizer", "Ztišit tohoto organizátora"),
    ("Restore organizer priority", "Obnovit prioritu organizátora"),
    ("Quiet this series", "Ztišit tuto sérii"),
    ("Restore this series", "Obnovit tuto sérii"),
    ("Quiet this invitation", "Ztišit tuto pozvánku"),
    ("Restore this invitation", "Obnovit tuto pozvánku"),
    ("Invitation priorities and my team", "Priority pozvánek a můj tým"),
    ("Calendar account changed. Close and reopen these settings.", "Přihlášení ke kalendáři se změnilo. Zavřete a znovu otevřete toto nastavení."),
    ("Customer matches use confirmed domains. A different domain alone is not proof of another tenant or a customer.", "Zákazníci se rozpoznávají podle potvrzených domén. Jiná doména sama o sobě nedokazuje jiný tenant ani zákazníka."),
    ("Team groups", "Týmové skupiny"),
    ("Search historical received mail for group names, one search per line.", "Vyhledat názvy skupin v dříve přijatých e-mailech, jeden dotaz na řádek."),
    ("Region STU\nRegion ATU\nRegion CSU", "Region STU\nRegion ATU\nRegion CSU"),
    ("Find groups in historical mail", "Najít skupiny v historii e-mailů"),
    ("Refresh team members", "Obnovit členy týmu"),
    ("Discovery samples up to 25 messages per search. Select only groups that really represent your close team; membership refreshes daily.", "Vyhledávání prochází vzorek nejvýše 25 zpráv na dotaz. Vyberte pouze skupiny svého nejužšího týmu; členství se obnovuje denně."),
    ("Cached email aliases", "Uložené e-mailové aliasy"),
    ("Membership not loaded yet", "Členství zatím nebylo načteno"),
    ("Remove from my team", "Odebrat z mého týmu"),
    ("People and exceptions", "Lidé a výjimky"),
    ("One email address per line. Explicit exclusions override group membership.", "Jedna e-mailová adresa na řádek. Výjimky mají přednost před členstvím ve skupinách."),
    ("Include in my close team", "Zahrnout do mého nejužšího týmu"),
    ("Exclude from team priority", "Neupřednostňovat jako člena týmu"),
    ("Save people and exceptions", "Uložit lidi a výjimky"),
    ("Organizer rules", "Pravidla organizátorů"),
    ("These priorities and caches stay in this Tomlook profile. No meeting response, message or directory change is sent.", "Priority a cache zůstávají pouze v tomto profilu Tomlooku. Neodesílá se odpověď na schůzku, zpráva ani změna adresáře."),
    ("Team list contains an invalid email address", "Seznam týmu obsahuje neplatnou e-mailovou adresu"),
    ("Some team groups could not be refreshed; saved members were kept", "Některé skupiny se nepodařilo obnovit; uložené členství zůstalo zachováno"),
    ("Invitation read timed out; cached data was preserved", "Načtení pozvánek překročilo časový limit; uložená data zůstala zachována"),
    ("WorkIQ disconnected during invitation read; cache preserved", "WorkIQ se během čtení pozvánek odpojilo; cache zůstala zachována"),
    ("Invitation refresh exceeded its three-minute deadline; cache preserved", "Obnova pozvánek překročila tříminutový limit; cache zůstala zachována"),
    ("Settings", "Nastavení"),
    ("MSX: not connected", "MSX: nepřipojeno"),
    ("MSX: checking...", "MSX: ověřování..."),
    ("MSX: signing in...", "MSX: přihlašování..."),
    ("MSX: connected", "MSX: připojeno"),
    ("MSX: sign-in required", "MSX: nutné přihlášení"),
    ("MSX: unavailable", "MSX: nedostupné"),
    ("MSX: off", "MSX: vypnuto"),
    ("Checking...", "Ověřování..."),
    ("Signing in...", "Přihlašování..."),
    ("Sign-in required", "Nutné přihlášení"),
    ("Off", "Vypnuto"),
    ("MSX", "MSX"),
    ("MSX connection", "Připojení MSX"),
    ("MSX · Dataverse and Power BI", "MSX · Dataverse a Power BI"),
    ("Microsoft tenant", "Tenant Microsoft"),
    ("Signed-in Azure account", "Přihlášený účet Azure"),
    ("Not known yet", "Zatím neznámý"),
    ("Dataverse (MSX)", "Dataverse (MSX)"),
    ("Power BI (Azure consumption)", "Power BI (spotřeba Azure)"),
    ("Sign in to MSX", "Přihlásit se k MSX"),
    ("Use device code", "Použít kód zařízení"),
    ("Check connection", "Ověřit připojení"),
    ("Disconnect", "Odpojit"),
    (
        "Opens Microsoft sign-in through Azure CLI (browser or Windows prompt).",
        "Otevře přihlášení Microsoft přes Azure CLI (v prohlížeči nebo v okně Windows).",
    ),
    (
        "Alternative sign-in with a code on another device; your tenant may block it.",
        "Náhradní přihlášení kódem na jiném zařízení; váš tenant ho může blokovat.",
    ),
    (
        "Forgets Tomlook's in-memory MSX tokens; your Azure CLI sign-in stays.",
        "Zapomene tokeny MSX uložené v paměti Tomlooku; přihlášení v Azure CLI zůstane.",
    ),
    (
        "Uses your Azure CLI sign-in; may change Azure CLI's default account.",
        "Používá vaše přihlášení v Azure CLI; může změnit výchozí účet Azure CLI.",
    ),
    (
        "Read-only: Tomlook never creates or changes MSX records. Tokens stay in memory.",
        "Jen pro čtení: Tomlook v MSX nikdy nic nevytváří ani nemění. Tokeny zůstávají jen v paměti.",
    ),
    ("MSX and Azure consumption", "MSX a spotřeba Azure"),
    (
        "Read-only customer context from MSX (Dataverse) and Azure consumption (Power BI).",
        "Přehled o zákaznících z MSX (Dataverse) a spotřeba Azure (Power BI), jen pro čtení.",
    ),
    ("Use MSX and Azure consumption", "Používat MSX a spotřebu Azure"),
    (
        "Tomlook stays idle until you sign in or Azure CLI already has a token. It never opens a sign-in prompt on its own.",
        "Tomlook nic nedělá, dokud se nepřihlásíte nebo dokud Azure CLI nemá platný token. Přihlašovací okno sám nikdy neotevře.",
    ),
    ("Microsoft tenant ID", "ID tenanta Microsoft"),
    ("Save tenant", "Uložit tenanta"),
    ("Open MSX connection", "Otevřít připojení MSX"),
    ("Reading MSX (Dataverse)", "Čtu MSX (Dataverse)"),
    (
        "Reading Azure consumption (Power BI)",
        "Čtu spotřebu Azure (Power BI)",
    ),
    (
        "Not connected. MSX stays idle until you sign in.",
        "Nepřipojeno. MSX nic nedělá, dokud se nepřihlásíte.",
    ),
    ("Checking the MSX connection...", "Ověřuji připojení k MSX..."),
    (
        "Connected read-only to Dataverse and Power BI.",
        "Připojeno k Dataverse a Power BI, jen pro čtení.",
    ),
    (
        "Dataverse connected; Azure consumption (Power BI) is unavailable.",
        "Dataverse je připojené; spotřeba Azure (Power BI) není k dispozici.",
    ),
    (
        "Power BI connected; Dataverse is unavailable.",
        "Power BI je připojené; Dataverse není k dispozici.",
    ),
    ("MSX is turned off in Settings.", "MSX je vypnuté v Nastavení."),
    (
        "Offline demo never connects MSX.",
        "Offline ukázka se k MSX nikdy nepřipojuje.",
    ),
    (
        "Disconnected. In-memory MSX tokens were forgotten; your Azure CLI sign-in is unchanged.",
        "Odpojeno. Tokeny MSX z paměti byly zapomenuty; přihlášení v Azure CLI se nezměnilo.",
    ),
    (
        "Complete the Microsoft sign-in that Azure CLI opened (browser or Windows prompt).",
        "Dokončete přihlášení Microsoft, které otevřelo Azure CLI (v prohlížeči nebo v okně Windows).",
    ),
    (
        "Requesting a device code from Azure CLI (up to 60 seconds)...",
        "Čekám na kód zařízení od Azure CLI (nejvýše 60 sekund)...",
    ),
    ("MSX check cancelled.", "Ověření MSX bylo zrušeno."),
    (
        "The MSX tenant must be a Microsoft Entra tenant ID (GUID).",
        "Tenant pro MSX musí být ID tenanta Microsoft Entra (GUID).",
    ),
    ("Connected (read-only).", "Připojeno (jen pro čtení)."),
    ("Dataverse read timed out", "Čtení z Dataverse vypršelo"),
    ("Power BI read timed out", "Čtení z Power BI vypršelo"),
    (
        "MSX sign-in required. Choose Sign in to MSX; Tomlook never opens a sign-in prompt on its own.",
        "Je nutné přihlášení k MSX. Zvolte Přihlásit se k MSX; Tomlook sám přihlašovací okno nikdy neotevře.",
    ),
    (
        "MSX sign-in required: Azure CLI did not return a cached token within 20 seconds.",
        "Je nutné přihlášení k MSX: Azure CLI do 20 sekund nevrátilo uložený token.",
    ),
    (
        "Azure CLI is not installed. Install the Azure CLI (MSI) to connect MSX.",
        "Azure CLI není nainstalované. Pro připojení k MSX nainstalujte Azure CLI (MSI).",
    ),
    (
        "Azure CLI could not start. Check its installation.",
        "Azure CLI se nepodařilo spustit. Zkontrolujte jeho instalaci.",
    ),
    (
        "Azure CLI returned a token for a different tenant; MSX was not connected.",
        "Azure CLI vrátilo token pro jiného tenanta; MSX nebylo připojeno.",
    ),
    (
        "Azure CLI returned an unusable token response; MSX was not connected.",
        "Azure CLI vrátilo nepoužitelnou odpověď s tokenem; MSX nebylo připojeno.",
    ),
    (
        "Azure CLI could not reach Microsoft sign-in; check connectivity. Retrying automatically.",
        "Azure CLI se nespojilo s přihlašovací službou Microsoft; zkontrolujte připojení. Pokus se automaticky zopakuje.",
    ),
    (
        "MSX sign-in failed or was blocked by tenant policy. Try again or use the other sign-in method.",
        "Přihlášení k MSX se nezdařilo nebo ho zablokovaly zásady tenanta. Zkuste to znovu nebo použijte druhý způsob přihlášení.",
    ),
    ("MSX sign-in cancelled.", "Přihlášení k MSX bylo zrušeno."),
    (
        "MSX sign-in timed out. Start sign-in again.",
        "Přihlášení k MSX vypršelo. Spusťte ho znovu.",
    ),
    (
        "Azure CLI did not provide a device code within 60 seconds. Your tenant may block device-code sign-in; use Sign in to MSX instead.",
        "Azure CLI do 60 sekund neposkytlo kód zařízení. Váš tenant může přihlášení kódem blokovat; použijte místo toho Přihlásit se k MSX.",
    ),
    ("Foundry: not checked", "Foundry: neověřeno"),
    ("Foundry: checking...", "Foundry: ověřování..."),
    ("Foundry: signed in", "Foundry: přihlášeno"),
    ("Foundry: sign-in required", "Foundry: nutné přihlášení"),
    ("Foundry: not configured", "Foundry: nenastaveno"),
    ("Foundry: unavailable", "Foundry: nedostupné"),
    ("Foundry: signing in...", "Foundry: přihlašování..."),
    (
        "Azure sign-in verified. Deployment access is checked when Auto runs.",
        "Přihlášení Azure je ověřené. Přístup k nasazenému modelu se ověří při použití Auto.",
    ),
    (
        "Checking Foundry Azure sign-in...",
        "Ověřuje se přihlášení Azure pro Foundry...",
    ),
    (
        "Requesting an Azure device code (up to 60 seconds)...",
        "Čeká se na přihlašovací kód od Azure (nejvýše 60 sekund)...",
    ),
    (
        "Enter this code on Microsoft's device sign-in page.",
        "Zadejte tento kód na přihlašovací stránce Microsoftu.",
    ),
    ("Copy code", "Kopírovat kód"),
    (
        "Open Microsoft device sign-in",
        "Otevřít přihlášení Microsoft",
    ),
    ("Cancel Azure sign-in", "Zrušit přihlášení Azure"),
    (
        "Sign in to Azure (device code)",
        "Přihlásit Azure pomocí kódu",
    ),
    ("Check Foundry connection", "Ověřit připojení Foundry"),
    ("Fix Foundry connection", "Vyřešit připojení Foundry"),
    (
        "GitHub Copilot generates answers and analyses. Foundry selects Quick or Thorough in Auto mode.",
        "GitHub Copilot vytváří odpovědi a analýzy. Foundry v režimu Auto vybírá mezi Rychle a Důkladně.",
    ),
    (
        "Uses your Azure CLI profile; its default account may change during sign-in.",
        "Používá váš profil Azure CLI; přihlášení může změnit jeho výchozí účet.",
    ),
    (
        "Azure device-code sign-in cancelled. No question was retried.",
        "Přihlášení Azure bylo zrušeno. Žádný dotaz nebyl znovu odeslán.",
    ),
    (
        "Azure device-code sign-in timed out. Start sign-in again for a new code.",
        "Platnost přihlášení Azure vypršela. Nový kód získáte opětovným spuštěním přihlášení.",
    ),
    (
        "Azure device-code sign-in failed or was blocked by tenant policy. Check the configured tenant and try again.",
        "Přihlášení Azure se nezdařilo nebo ho blokují pravidla tenantu. Zkontrolujte nastavený tenant a zkuste to znovu.",
    ),
    (
        "Azure CLI did not provide a device code within 60 seconds. Check connectivity and try signing in again.",
        "Azure CLI nevydalo přihlašovací kód do 60 sekund. Zkontrolujte připojení a spusťte přihlášení znovu.",
    ),
    (
        "Azure sign-in code could not reach the interface. Try signing in again.",
        "Přihlašovací kód Azure se nepodařilo předat do okna aplikace. Spusťte přihlášení znovu.",
    ),
    (
        "Configured Azure CLI could not start. Check its installation and decision-router.json.",
        "Nastavené Azure CLI nelze spustit. Zkontrolujte instalaci a decision-router.json.",
    ),
    (
        "Azure sign-in check timed out. Check connectivity and retry.",
        "Ověření přihlášení Azure překročilo časový limit. Zkontrolujte připojení a zkuste to znovu.",
    ),
    (
        "Offline demo never opens Azure sign-in.",
        "Offline ukázka nikdy nespouští přihlášení Azure.",
    ),
    (
        "Retrying isolated AI connection...",
        "Obnovuje se izolované připojení AI...",
    ),
    (
        "WorkIQ connection interrupted.",
        "Připojení WorkIQ bylo přerušeno.",
    ),
    ("Display", "Zobrazení"),
    ("Language", "Jazyk"),
    ("Theme", "Motiv"),
    (
        "Calendar provider returned a different meeting; no write was submitted",
        "Poskytovatel kalendáře vrátil jinou schůzku. Žádný zápis nebyl odeslán.",
    ),
    ("AI prompts", "AI prompty"),
    (
        "Personal guidance only; evidence, privacy and write-safety rules stay enforced.",
        "Osobní pokyny pro AI; pravidla pro podklady, soukromí a bezpečné zápisy zůstávají platná.",
    ),
    ("Before the meeting", "Před schůzkou"),
    ("After the meeting", "Po schůzce"),
    ("Quick answers", "Rychlé odpovědi"),
    ("Thorough research", "Důkladné zjišťování"),
    ("Workplace research", "Pracovní kontext"),
    ("Save prompts", "Uložit prompty"),
    ("Restore default", "Obnovit výchozí"),
    (
        "Saved notes are not reprocessed when prompts change.",
        "Změna promptů nezpracovává již uložené zápisy znovu.",
    ),
    (
        "Each prompt needs 1 to 4000 UTF-8 bytes and no unsupported control characters",
        "Každý prompt musí obsahovat 1 až 4000 bajtů UTF-8 a nesmí obsahovat nepodporované řídicí znaky.",
    ),
    ("Previous month", "Předchozí měsíc"),
    ("Next month", "Následující měsíc"),
    ("Follow", "Sledovat"),
    ("Stop following", "Přestat sledovat"),
    (
        "Follow is for invitations, not meetings you organize",
        "Follow lze použít u pozvánek, ne u schůzek, které organizujete.",
    ),
    (
        "Refresh the calendar first; Follow needs verified invitation details",
        "Nejprve obnovte kalendář. Follow vyžaduje ověřené údaje o pozvánce.",
    ),
    ("This meeting is not followed", "Tato schůzka nemá Follow."),
    (
        "Follow requires room for one more calendar category",
        "Pro Follow je potřeba místo na jednu další kategorii.",
    ),
    (
        "Adds Follow, keeps other categories and marks this occurrence as free. Your invitation response is unchanged; the organizer is not notified.",
        "Přidá kategorii Follow, zachová ostatní kategorie a označí tento výskyt jako volno. Odpověď na pozvánku se nemění a organizátor nedostane oznámení.",
    ),
    (
        "Removes Follow only. Availability stays unchanged; the organizer is not notified.",
        "Odebere pouze kategorii Follow. Dostupnost se nemění a organizátor nedostane oznámení.",
    ),
    ("Meeting ended", "Schůzka již proběhla"),
    ("Meeting notes", "Zápis ze schůzky"),
    ("Check transcript", "Zkontrolovat přepis"),
    (
        "No AI meeting notes yet.",
        "AI zápis zatím není k dispozici.",
    ),
    ("AI meeting notes ready", "AI zápis je připraven"),
    ("AI preparation ready", "AI příprava je připravena"),
    ("My tasks", "Moje úkoly"),
    ("Mentions of me", "Zmínky o mně"),
    (
        "Technical takeaways and materials",
        "Technické poznatky a materiály",
    ),
    (
        "Decisions and unresolved issues",
        "Rozhodnutí a otevřené body",
    ),
    ("Original meeting source", "Původní zdroj schůzky"),
    ("Required", "Povinný"),
    ("Optional", "Nepovinný"),
    ("Resource", "Prostředek"),
    ("Role unavailable", "Role není dostupná"),
    (
        "Previous attempt; no calendar change was sent.",
        "Předchozí pokus; žádná změna kalendáře nebyla odeslána.",
    ),
    ("Retry calendar change", "Zkusit změnu znovu"),
    ("Review changes", "Zkontrolovat změny"),
    ("Applied changes", "Provedené změny"),
    ("Pending change proposal", "Návrh změny ke schválení"),
    ("Approve change", "Schválit změnu"),
    ("Proposed event", "Návrh události"),
    ("Calendar", "Kalendář"),
    ("No category", "Bez kategorie"),
    ("Keep current category", "Ponechat současnou kategorii"),
    ("A suggested category is only applied after you confirm.", "Navržená kategorie se nastaví až po potvrzení."),
    ("A category can only be combined with accepting an invitation", "Kategorii lze současně nastavit pouze při přijetí pozvánky"),
    ("Category assignment could not be verified; meeting acceptance was not attempted.", "Nastavení kategorie se nepodařilo ověřit; přijetí schůzky nebylo zahájeno."),
    ("The category was assigned, but meeting acceptance was not submitted. Inspect Outlook; this approval will not be replayed automatically.", "Kategorie byla nastavena, ale přijetí schůzky nebylo odesláno. Zkontrolujte Outlook; toto schválení se automaticky nezopakuje."),
    ("The category was assigned, but meeting acceptance could not be verified. Inspect Outlook; this approval will not be replayed automatically.", "Kategorie byla nastavena, ale přijetí schůzky se nepodařilo ověřit. Zkontrolujte Outlook; toto schválení se automaticky nezopakuje."),
    ("The category was assigned, but the meeting changed before acceptance could be verified. Inspect Outlook; this approval will not be replayed automatically.", "Kategorie byla nastavena, ale schůzka se změnila dřív, než šlo přijetí ověřit. Zkontrolujte Outlook; toto schválení se automaticky nezopakuje."),
    ("Prepare for this meeting", "Připravit se na schůzku"),
    ("Check BLOCKER", "Ověřit blocker"),
    ("Run /prepare in Ask AI with this meeting as context", "Spustí /prepare v Ask AI s touto schůzkou jako kontextem"),
    ("Run /blocker in Ask AI with this meeting as context", "Spustí /blocker v Ask AI s touto schůzkou jako kontextem"),
    ("Check BLOCKERs, final invitations, email and Teams", "Prověřit blockery, finální pozvánky, e-maily a Teams"),
    ("Choose a meeting context or add search terms after /blocker.", "Vyberte schůzku jako kontext nebo za /blocker doplňte hledaná slova."),
    (crate::ask::BLOCKER_PROMPT,
        "Prověř, jestli mám kontextový BLOCKER (nebo rezervaci určenou zadanými hledanými slovy) dál držet. Jde o rešerši, ne o povolení měnit kalendář nebo odesílat zprávy.\n\
Urči přesnou rezervaci a uveď celé místní datum a čas, organizátora a známé účastníky. Podle potřeby přečti původní popis; z názvu nevyvozuj finální rozhodnutí. Pokud je výběr nejednoznačný, požádej o upřesnění místo odhadu.\n\
V ohraničeném relevantním období kalendáře najdi další související výskyty BLOCKERů i skutečné pozvánky, včetně možného přesunu termínu. Každý nalezený záznam uveď samostatně s datem, časem a identitou. Porovnej téma, organizátora, účastníky a podklady; podobný název ani sousední čas samy nepotvrzují, že události patří k sobě. Uveď prohledané období a mezery v synchronizaci kalendáře.\n\
Prohledej e-maily i Teams: debaty o termínech, hlasování, rozhodnutí organizátora, potvrzení a zrušení. Dej přednost novějším původním zprávám a aktuální pozvánce před starými plány. Rozhodující tvrzení ověř cíleným čtením originálů a dolož vrácenými odkazy a daty zpráv. Souhrn vyhledávání ani samotný odkaz nejsou ověřeným obsahem originálu. Rozliš nedostupný zdroj od hledání bez výsledků; nevymýšlej rozhodnutí z Teams ani finální termín.\n\
Pro každou související rezervaci zvlášť uzavři: ponechat, pravděpodobně zbytečná díky ověřené finální pozvánce, nebo nejasné. Pokud finální pozvánka existuje, uveď celé datum a čas, stav zrušení a odpovědi. Vysvětli podklady a nejistotu; nenalezení podkladů není důvod ke smazání blockeru. Finální pozvánku a nesouvisející události zachovej.\n\
Pokud výsledek není jasný, podle ověřených podkladů označ organizátora, vlastníka hlasování nebo odesílatele relevantní zprávy, na koho se obrátit a proč. Připrav krátký návrh e-mailu nebo zprávy do Teams v jazyce uživatele s dotazem, zda mám konkrétní rezervaci dál držet, či uvolnit, a který termín platí. Odvozené kontakty a neznámé adresy označ; příjemce neodhaduj a nic neposílej.\n\
Vrať stručný závěr, datovaný seznam rezervací a finálních pozvánek, důležité podklady z e-mailů a Teams, mezery a jeden konkrétní další krok. Přesné položky k úklidu doporuč jen s oporou v podkladech. Návrhy smazání nezakládej, dokud uživatel výslovně nepožádá o úklid; potom pro každý ověřený výskyt použij nástroj pro čekající návrh a vyžádej schválení v Tomlooku. Z chatu nikdy neprováděj ani neschvaluj změny."),
    ("The proposed event starts in the past; choose a future time", "Navržená událost začíná v minulosti. Zvolte budoucí čas."),
    ("Event proposal is no longer pending", "Návrh události už nečeká na schválení"),
    ("Event proposal dismissed", "Návrh události zamítnut"),
    ("Approve and create event", "Schválit a vytvořit událost"),
    ("Dismiss event proposal", "Zamítnout návrh události"),
    ("Event proposal dismissed; nothing was created", "Návrh události byl zamítnut. Nic se nevytvořilo."),
    ("Attendee availability has not been verified.", "Dostupnost účastníků nebyla ověřena."),
    ("Not private", "Není soukromá"),
    ("The organizer will not be notified.", "Organizátor nebude upozorněn."),
    ("No attendees will be notified.", "Žádní účastníci nebudou upozorněni."),
    ("Edit Insights", "Upravit Insights"),
    ("Edit meeting notes", "Upravit zápis"),
    ("Edit meeting text", "Upravit text schůzky"),
    ("Add travel buffer", "Přidat přesun"),
    ("Before", "Před změnou"),
    ("After", "Po změně"),
    ("Append to description", "Doplnit do popisu"),
    ("Current description", "Současný popis"),
    ("Preparing a change for approval", "Připravuji změnu ke schválení"),
    ("Review AI-proposed edits in Messages.", "Změny navržené AI zkontrolujte ve Zprávách."),
    ("Updates both languages; untouched sections and original evidence stay unchanged.",
        "Změna se uloží v obou jazycích. Ostatní sekce a původní zdroje zůstanou beze změny."),
    ("Approved sections are preserved during automatic updates.",
        "Schválené sekce zůstanou zachované i při automatické aktualizaci."),
    ("Changing meeting text can notify attendees.", "Změna textu schůzky může účastníkům odeslat oznámení."),
    ("AI proposal awaiting your approval; nothing has been changed",
        "Návrh AI čeká na vaše schválení. Zatím se nic nezměnilo."),
    ("Approved meeting notes saved", "Schválené úpravy zápisu jsou uložené"),
    ("Approved edits saved; changed sections are preserved during future AI updates",
        "Schválené změny jsou uložené. Upravené sekce zůstanou zachované při dalších aktualizacích AI."),
    ("Saved document changed; the outdated proposal was removed. Ask for a new proposal.",
        "Uložený dokument se změnil. Zastaralý návrh byl odebrán; požádejte o nový."),
    (
        "Checking transcript and meeting chat",
        "Kontroluji přepis a chat schůzky",
    ),
    (
        "Waiting for a transcript (up to 7 days after the meeting)",
        "Čeká na přepis, nejdéle 7 dní po schůzce",
    ),
    (
        "Automatic notes cover accepted or followed online meetings from the last 7 days.",
        "Automatické zápisy zahrnují přijaté nebo sledované online schůzky za posledních 7 dní.",
    ),
    (
        "Meeting notes require an accepted or followed online meeting that ended within the last 7 days",
        "Zápis vyžaduje přijatou nebo sledovanou online schůzku, která skončila během posledních 7 dní.",
    ),
    (
        "AI notes from the original transcript and available meeting chat; verify assignments against sources",
        "AI zápis z původního přepisu a dostupného chatu; úkoly ověřte v původních zdrojích",
    ),
    (
        "Read-only research interrupted; other meetings can continue",
        "Zjišťování bylo přerušeno; ostatní schůzky se mohou zpracovávat dál",
    ),
    ("Create travel blocks", "Vytvořit přesuny"),
    ("Add travel blocks", "Přidat přejezdy"),
    ("Travel blocks require your own timed meeting or an accepted invitation", "Přejezdy vyžadují vlastní časovanou schůzku nebo přijatou pozvánku"),
    ("Creating travel blocks", "Vytváření přesunů"),
    ("Travel blocks created", "Přesuny vytvořeny"),
    ("Travel blocks not created", "Přesuny nebyly vytvořeny"),
    (
        "Check travel blocks in Outlook",
        "Zkontrolujte přesuny v Outlooku",
    ),
    (
        "The existing meeting stays unchanged; only two travel blocks are created.",
        "Původní schůzka zůstane beze změny; vytvoří se pouze dva přesuny.",
    ),
    (
        "Travel blocks require your own timed meeting",
        "Přesuny lze přidat pouze k vlastní schůzce s konkrétním časem",
    ),
    (
        "Refresh the calendar first; the meeting's privacy is unavailable",
        "Nejprve obnovte kalendář; soukromí schůzky zatím není známé",
    ),
    ("Travel time is unavailable", "Délka přesunu není dostupná"),
    ("New event", "Nová událost"),
    (
        "Click or drag an empty time in the calendar",
        "Klikněte do volného času nebo vyberte interval tažením myši",
    ),
    ("Event title", "Název události"),
    ("Start", "Začátek"),
    ("End", "Konec"),
    (
        "Local time; 15-minute steps. 24:00 is the next midnight.",
        "Místní čas, kroky po 15 minutách. 24:00 je půlnoc následujícího dne.",
    ),
    ("Description (optional)", "Popis (nepovinný)"),
    ("Private event", "Soukromá událost"),
    (
        "Private in Outlook; delegates allowed to view private items may still see details.",
        "Soukromá v Outlooku; delegáti s oprávněním zobrazovat soukromé položky mohou podrobnosti stále vidět.",
    ),
    (
        "No personal category was found; choose a category above.",
        "Osobní kategorie nebyla nalezena; vyberte kategorii nahoře.",
    ),
    ("Teams meeting", "Schůzka v Teams"),
    ("Join Teams meeting", "Připojit se v Teams"),
    (
        "Add travel blocks before and after",
        "Přidat přesun před událostí i po ní",
    ),
    ("Each travel block", "Délka každého přesunu"),
    ("Travel to meeting", "Přesun na schůzku"),
    ("Travel from meeting", "Přesun ze schůzky"),
    (
        "Creates busy appointments in your calendar. No attendees or room invitations.",
        "Vytvoří blokace času ve vašem kalendáři. Bez účastníků a rezervace místností.",
    ),
    (
        "Not synced; availability has not been verified.",
        "Není synchronizováno; dostupnost času nebyla ověřena.",
    ),
    ("Overlaps saved events", "Překrývá uložené události"),
    ("Create event", "Vytvořit událost"),
    ("Create and send invitations", "Vytvořit a odeslat pozvánky"),
    ("Attendees", "Účastníci"),
    ("Name or e-mail", "Jméno nebo e-mail"),
    (
        "Switch between required and optional",
        "Přepnout mezi povinnou a nepovinnou účastí",
    ),
    ("Remove attendee", "Odebrat účastníka"),
    ("Searching the directory...", "Hledám v adresáři…"),
    ("Directory search is unavailable", "Hledání v adresáři není dostupné"),
    ("Invitations are unavailable", "Pozvánky nejsou dostupné"),
    (
        "Invitation support could not be checked",
        "Podporu pozvánek se nepodařilo ověřit",
    ),
    ("Invited", "Pozváni"),
    (
        "You are the organizer and are not invited",
        "Jste organizátor, sami sebe nepozýváte",
    ),
    ("Already invited", "Už je mezi pozvanými"),
    (
        "Enter a valid e-mail address or choose a suggestion",
        "Zadejte platnou e-mailovou adresu nebo vyberte návrh",
    ),
    (
        "A new event can invite at most 50 attendees",
        "Na novou událost lze pozvat nejvýše 50 účastníků",
    ),
    (
        "An attendee name is too long or contains unsupported characters",
        "Jméno účastníka je příliš dlouhé nebo obsahuje nepodporované znaky",
    ),
    (
        "Each attendee can be invited only once",
        "Každého účastníka lze pozvat jen jednou",
    ),
    (
        "Your own mailbox is the organizer and cannot be invited",
        "Vaše vlastní schránka je organizátor a nelze ji pozvat",
    ),
    (
        "Your calendar provider does not accept attendees on new events through Tomlook; remove the attendees or invite them in Outlook",
        "Váš poskytovatel kalendáře nepřijímá účastníky nových událostí z Tomlooku; odeberte účastníky nebo je pozvěte v Outlooku",
    ),
    (
        "Invitations may already have been sent; check Outlook before retrying.",
        "Pozvánky už mohly být odeslány; před dalším pokusem zkontrolujte Outlook.",
    ),
    (
        "All requested events were created and verified; Outlook sent the invitations",
        "Všechny požadované události byly vytvořeny a ověřeny; Outlook odeslal pozvánky",
    ),
    (
        "Connect WorkIQ to see attendee availability.",
        "Pro zobrazení dostupnosti účastníků připojte WorkIQ.",
    ),
    ("Checking availability...", "Zjišťuji dostupnost…"),
    (
        "Availability could not be checked",
        "Dostupnost se nepodařilo zjistit",
    ),
    (
        "You can still create the event; availability is not verified.",
        "Událost můžete vytvořit i tak; dostupnost ale není ověřená.",
    ),
    ("Suggest times", "Navrhnout termíny"),
    (
        "Find times around this day when attendees are free",
        "Najít termíny kolem tohoto dne, kdy mají účastníci volno",
    ),
    (
        "Meeting times could not be suggested",
        "Termíny se nepodařilo navrhnout",
    ),
    (
        "Click to start the meeting here",
        "Kliknutím sem přesunete začátek schůzky",
    ),
    ("Free", "Volno"),
    ("Busy", "Obsazeno"),
    ("Out of office", "Mimo kancelář"),
    ("Working elsewhere", "Pracuje jinde"),
    ("Unknown", "Neznámé"),
    (
        "Not found in your organization's directory",
        "Nenalezeno v adresáři vaší organizace",
    ),
    (
        "Availability is not shared with you",
        "Dostupnost s vámi není sdílená",
    ),
    ("Availability is not visible", "Dostupnost není vidět"),
    ("No availability was returned", "Dostupnost nebyla vrácena"),
    (
        "Availability can be checked for 1 to 51 people",
        "Dostupnost lze zjistit pro 1 až 51 osob",
    ),
    (
        "Availability needs valid, unique e-mail addresses",
        "Pro zjištění dostupnosti jsou potřeba platné a neopakující se e-mailové adresy",
    ),
    (
        "Availability covers at most 22 days",
        "Dostupnost lze zjistit nejvýše pro 22 dní",
    ),
    (
        "Local midnight is missing or ambiguous because of daylight saving time",
        "Místní půlnoc neexistuje nebo je nejednoznačná kvůli změně letního času",
    ),
    (
        "Invalid availability date",
        "Neplatné datum pro zjištění dostupnosti",
    ),
    (
        "Availability response has no schedule list",
        "Odpověď o dostupnosti neobsahuje seznam kalendářů",
    ),
    (
        "Availability response exceeded its schedule limit",
        "Odpověď o dostupnosti překročila limit počtu kalendářů",
    ),
    (
        "Availability response is not valid JSON",
        "Odpověď o dostupnosti není platný JSON",
    ),
    (
        "WorkIQ does not offer availability lookups on this connection",
        "WorkIQ v tomto připojení nenabízí zjišťování dostupnosti",
    ),
    (
        "Availability lookups are not permitted for this account",
        "Zjišťování dostupnosti není pro tento účet povoleno",
    ),
    (
        "Availability lookup returned a tool error; provider details withheld",
        "Zjišťování dostupnosti skončilo chybou nástroje; podrobnosti poskytovatele nejsou zobrazeny",
    ),
    (
        "Availability lookup exceeded its 30-second deadline",
        "Zjišťování dostupnosti překročilo limit 30 sekund",
    ),
    (
        "Availability lookup could not be queued; try again",
        "Zjištění dostupnosti se nepodařilo zařadit; zkuste to znovu",
    ),
    (
        "Connect WorkIQ to look up people and availability",
        "Pro hledání lidí a zjištění dostupnosti připojte WorkIQ",
    ),
    ("Superseded by a newer lookup", "Nahrazeno novějším dotazem"),
    (
        "Lookup cancelled before it was sent",
        "Dotaz byl zrušen před odesláním",
    ),
    ("Tomlook is stopping", "Tomlook se ukončuje"),
    (
        "WorkIQ authorization expired; sign in again",
        "Přihlášení k WorkIQ vypršelo; přihlaste se znovu",
    ),
    (
        "People search needs 2 to 100 characters",
        "Hledání lidí potřebuje 2 až 100 znaků",
    ),
    (
        "People search did not return a list",
        "Hledání lidí nevrátilo seznam",
    ),
    (
        "People search could not be queued; try again",
        "Hledání lidí se nepodařilo zařadit; zkuste to znovu",
    ),
    (
        "Add attendees before suggesting times",
        "Před návrhem termínů přidejte účastníky",
    ),
    (
        "Meeting time suggestions need a 15-minute-step duration within five days",
        "Návrh termínů vyžaduje délku v krocích po 15 minutách v rozsahu nejvýše pěti dnů",
    ),
    (
        "Choose a day that is not in the past to find meeting times",
        "Pro návrh termínů vyberte den, který není v minulosti",
    ),
    (
        "Meeting time suggestions are unavailable",
        "Návrhy termínů nejsou dostupné",
    ),
    (
        "Meeting time suggestions could not be queued; try again",
        "Návrh termínů se nepodařilo zařadit; zkuste to znovu",
    ),
    (
        "Attendees are not available in this range",
        "Účastníci v tomto období nemají volno",
    ),
    (
        "Attendees are unavailable or their availability is unknown",
        "Účastníci nemají volno nebo jejich dostupnost není známá",
    ),
    (
        "You are not available in this range",
        "V tomto období nemáte volno",
    ),
    (
        "No meeting times were found in this range",
        "V tomto období se nenašly žádné vhodné termíny",
    ),
    ("Invalid current time", "Neplatný aktuální čas"),
    ("Event creation", "Vytváření událostí"),
    ("Creating event", "Vytváření události"),
    ("Event created", "Událost vytvořena"),
    ("Event not created", "Událost nebyla vytvořena"),
    (
        "Check event creation in Outlook",
        "Zkontrolujte vytvoření v Outlooku",
    ),
    (
        "All requested events were created and verified",
        "Všechny požadované události byly vytvořeny a ověřeny",
    ),
    (
        "Enter an event title (up to 1000 bytes)",
        "Zadejte název události (nejvýše 1000 bajtů)",
    ),
    (
        "The description is too long or contains unsupported characters",
        "Popis je příliš dlouhý nebo obsahuje nepodporované znaky",
    ),
    (
        "The event must end after it starts and last at most 24 hours",
        "Konec události musí být po začátku; nejdelší doba je 24 hodin",
    ),
    (
        "Choose an existing category",
        "Vyberte existující kategorii",
    ),
    (
        "Travel time must be 15 to 180 minutes in 15-minute steps",
        "Přesun musí trvat 15 až 180 minut v krocích po 15 minutách",
    ),
    (
        "Invalid event date or time",
        "Neplatné datum nebo čas události",
    ),
    (
        "This local time is missing or ambiguous because of daylight saving time",
        "Tento místní čas neexistuje nebo je nejednoznačný kvůli změně letního času",
    ),
    (
        "Event creation could not be queued; nothing was submitted",
        "Vytváření události se nepodařilo zařadit; nic nebylo odesláno",
    ),
    (
        "Calendar account or event creation approval changed",
        "Změnil se účet kalendáře nebo potvrzená událost",
    ),
    (
        "Wait for the current event creation to finish",
        "Počkejte na dokončení vytvářené události",
    ),
    (
        "Event creation history reached its safety limit",
        "Historie vytváření událostí dosáhla bezpečnostního limitu",
    ),
    (
        "Event creation approval was already consumed or could not be saved",
        "Potvrzení této události již bylo použito nebo se ho nepodařilo uložit",
    ),
    (
        "Calendar provider has no compatible event creation schema; nothing was submitted",
        "Poskytovatel kalendáře nemá kompatibilní schéma pro vytvoření události; nic nebylo odesláno",
    ),
    (
        "This calendar does not support Teams meetings; turn off Teams or ask your administrator",
        "Tento kalendář nepodporuje schůzky v Teams; vypněte Teams nebo kontaktujte správce",
    ),
    (
        "Some events were created; remaining events were not completed. Inspect Outlook before creating anything again.",
        "Některé události byly vytvořeny, zbývající nebyly dokončeny. Před dalším vytvářením zkontrolujte Outlook.",
    ),
    (
        "Event creation could not be verified completely. Inspect Outlook; no write will be repeated automatically.",
        "Vytvoření události se nepodařilo úplně ověřit. Zkontrolujte Outlook; žádný zápis se nebude automaticky opakovat.",
    ),
    (
        "Automatic routing context exceeds its bounds; choose a mode explicitly",
        "Kontext automatického výběru překročil limit; vyberte režim ručně.",
    ),
    (
        "Configure Microsoft-Decision-1 in decision-router.json, or choose Quick or Thorough",
        "Nastavte Microsoft-Decision-1 v decision-router.json nebo zvolte Rychle či Důkladně.",
    ),
    (
        "Microsoft-Decision-1 needs a valid Azure CLI sign-in for the configured tenant and subscription; choose a mode explicitly",
        "Auto vyžaduje přihlášení Azure pro nastavený tenant a předplatné. Použijte přihlášení kódem nebo zvolte Rychle či Důkladně.",
    ),
    (
        "Microsoft-Decision-1 returned an invalid decision; choose a mode explicitly",
        "Microsoft-Decision-1 vrátil neplatné rozhodnutí; vyberte režim ručně.",
    ),
    (
        "Microsoft-Decision-1 routing timed out; choose a mode explicitly",
        "Vypršel čas pro výběr pomocí Microsoft-Decision-1; vyberte režim ručně.",
    ),
    (
        "Microsoft-Decision-1 connection failed; choose a mode explicitly",
        "Spojení s Microsoft-Decision-1 se nezdařilo; vyberte režim ručně.",
    ),
    (
        "Create Microsoft-Decision-1 connection failed",
        "Nepodařilo se vytvořit spojení s Microsoft-Decision-1.",
    ),
    (
        "Choosing a mode with Microsoft-Decision-1",
        "Vybírám režim pomocí Microsoft-Decision-1",
    ),
    ("Discard saved research", "Zahodit uložený výzkum"),
    ("Confirm discard", "Potvrdit zahození"),
    (
        "This permanently removes only these saved findings, not meetings or source documents.",
        "Trvale se odstraní pouze tato uložená zjištění, nikoli schůzky nebo zdrojové dokumenty.",
    ),
    (
        "This answer reached its remote-read budget",
        "Odpověď vyčerpala povolený počet vzdálených čtení.",
    ),
    (
        "This answer reached its local-read budget",
        "Odpověď vyčerpala povolený počet místních čtení.",
    ),
    (
        "AI execution capacity is busy; nothing was submitted",
        "Všechny povolené úlohy už běží; dotaz nebyl odeslán.",
    ),
    (
        "Automatic routing returned an invalid decision; choose a mode explicitly",
        "Automatický výběr vrátil neplatné rozhodnutí; vyberte režim ručně.",
    ),
    (
        "Automatic routing selected an unsupported mode; choose a mode explicitly",
        "Automatický výběr zvolil nepodporovaný režim; vyberte režim ručně.",
    ),
    (
        "Automatic routing session creation timed out; choose a mode explicitly",
        "Vypršel čas pro automatický výběr; vyberte režim ručně.",
    ),
    (
        "Creating the chat session timed out",
        "Vypršel čas pro vytvoření chatu.",
    ),
    (
        "Sending the chat request timed out",
        "Vypršel čas pro odeslání dotazu.",
    ),
    (
        "Copilot reported a chat session error; partial findings are not success",
        "Copilot ohlásil chybu chatu; průběžná zjištění nejsou dokončený výsledek.",
    ),
    (
        "Copilot completed without an answer",
        "Copilot skončil bez odpovědi.",
    ),
    (
        "This question was stopped; no further read was sent",
        "Dotaz byl zastaven; další čtení nebylo odesláno.",
    ),
    (
        "Conversation context exceeds its bounds; start a new chat",
        "Kontext rozhovoru dosáhl limitu; začněte nový chat.",
    ),
    ("New chat", "Nový chat"),
    ("Retry question", "Zkusit dotaz znovu"),
    (
        "This answer lane is busy; choose the other mode, wait or stop its question",
        "Tento režim už zpracovává dotaz; zvolte druhý, počkejte nebo dotaz zastavte.",
    ),
    (
        "A local command is already running; wait for its result",
        "Místní příkaz už běží; počkejte na jeho výsledek.",
    ),
    ("Quick", "Rychle"),
    ("Thorough", "Důkladně"),
    ("Send", "Odeslat"),
    ("You", "Vy"),
    ("Thinking", "Přemýšlím"),
    ("Your calendar, in conversation", "Váš kalendář v rozhovoru"),
    (
        "Ask naturally, or type / for instant local commands. Thorough research leaves the quick-answer lane free.",
        "Zeptejte se přirozeně nebo napište / pro okamžité místní příkazy. Během důkladného výzkumu můžete zadat rychlý dotaz.",
    ),
    (
        "Microsoft-Decision-1 selects Quick or Thorough; no local guessing",
        "Microsoft-Decision-1 vybere rychlou odpověď nebo důkladný výzkum; bez místního odhadování",
    ),
    (
        "GPT-6 Luna / low / targeted reads",
        "GPT-6 Luna / low / cílené hledání",
    ),
    (
        "GPT-6.1 Sol / medium / resumable research",
        "GPT-6.1 Sol / medium / výzkum s pokračováním",
    ),
    ("Activity", "Postup práce"),
    ("Open a meeting", "Otevřít schůzku"),
    ("Could not complete", "Nepodařilo se dokončit"),
    ("Interrupted research", "Přerušený výzkum"),
    ("Continue research", "Pokračovat ve výzkumu"),
    (
        "Continue from saved findings; the original provider call is not resumed.",
        "Pokračuje se z uložených zjištění, nikoli obnovením původního běhu u poskytovatele.",
    ),
    (
        "Today's saved calendar / no AI",
        "Dnešní uložený kalendář / bez AI",
    ),
    (
        "This week's saved calendar / no AI",
        "Uložený kalendář tohoto týdne / bez AI",
    ),
    (
        "Fuzzy search saved meetings / no AI",
        "Hledat i s překlepy v uložených schůzkách / bez AI",
    ),
    (
        "Read saved meeting Insights / no AI",
        "Přečíst uložený přehled schůzky / bez AI",
    ),
    ("Local calendar / no AI", "Místní kalendář / bez AI"),
    ("Local calendar", "Místní kalendář"),
    (
        "Saved Insights may be stale for this meeting.",
        "Uložený přehled nemusí odpovídat aktuální schůzce.",
    ),
    (
        "No saved Insights. Open the meeting to prepare them.",
        "Schůzka nemá uložený přehled. Otevřete ji pro jeho přípravu.",
    ),
    (
        "No matches in the saved calendar.",
        "V uloženém kalendáři nebyla nalezena shoda.",
    ),
    (
        "Showing the 25 best matches. Narrow /search to see more.",
        "Zobrazeno 25 nejlepších shod. Upřesněte hledání pomocí /search.",
    ),
    (
        "Approximate keyword match; verify the meeting.",
        "Přibližná shoda klíčových slov; ověřte správnou schůzku.",
    ),
    (
        "Cache coverage is incomplete; no matches do not prove there are no meetings.",
        "Místní kalendář nepokrývá celé období; prázdný výsledek neznamená, že nemáte schůzky.",
    ),
    ("Oldest calendar refresh", "Nejstarší aktualizace kalendáře"),
    (
        "Calendar refresh time is unknown.",
        "Čas aktualizace kalendáře není znám.",
    ),
    ("Preparing the request", "Připravuji požadavek"),
    ("Choosing a mode", "Vybírám režim"),
    ("Starting thorough research", "Spouštím důkladný výzkum"),
    ("Starting a quick answer", "Připravuji rychlou odpověď"),
    ("Writing the answer", "Píšu odpověď"),
    ("Searching saved meetings", "Hledám v uložených schůzkách"),
    ("Reading a saved meeting", "Čtu uloženou schůzku"),
    ("Reading saved Insights", "Čtu uložený přehled"),
    ("Searching local materials", "Hledám místní podklady"),
    ("Reading a local document", "Čtu místní dokument"),
    (
        "Reading mail or calendar in Microsoft 365",
        "Čtu poštu nebo kalendář v Microsoft 365",
    ),
    (
        "Searching Microsoft 365 context",
        "Hledám kontext v Microsoft 365",
    ),
    ("Reading cloud materials", "Čtu cloudové podklady"),
    ("Searching GitHub", "Hledám na GitHubu"),
    ("Searching public sources", "Hledám ve veřejných zdrojích"),
    ("Reading an authorized source", "Čtu povolený zdroj"),
    ("Saved calendar search", "Hledání v uloženém kalendáři"),
    ("Saved meeting", "Uložená schůzka"),
    ("Saved Insights", "Uložený přehled"),
    (
        "This chat holds 32 questions; start a new chat to continue",
        "Tento chat obsahuje 32 dotazů; pro další začněte nový chat.",
    ),
    (
        "Choose a meeting context before reading saved Insights",
        "Pro čtení uloženého přehledu zvolte kontext schůzky.",
    ),
    (
        "This local command takes no arguments. Use /search followed by meeting words.",
        "Tento místní příkaz nemá parametry. Pro hledání použijte /search a slova ze schůzky.",
    ),
    (
        "After /search, enter up to 200 characters from the meeting title, person or location.",
        "Za /search napište nejvýše 200 znaků z názvu schůzky, jména nebo místa.",
    ),
    (
        "Meeting is no longer in the local calendar; refresh and search again",
        "Schůzka už není v místním kalendáři; aktualizujte ho a hledejte znovu.",
    ),
    (
        "The cached calendar account changed; refresh before searching",
        "Účet místního kalendáře se změnil; před hledáním ho aktualizujte.",
    ),
    (
        "Research reached its ten-minute budget. Saved findings can be continued explicitly.",
        "Výzkum dosáhl desetiminutového limitu. Z uložených zjištění můžete výslovně pokračovat.",
    ),
    (
        "The quick-answer budget expired; retry or choose Thorough for a larger investigation.",
        "Vypršel limit rychlé odpovědi. Zkuste dotaz znovu nebo pro rozsáhlejší hledání zvolte Důkladně.",
    ),
    (
        "Stopped. Partial findings are not a completed answer.",
        "Zastaveno. Průběžná zjištění nejsou dokončenou odpovědí.",
    ),
    (
        "A thorough research is already running; quick questions remain available",
        "Důkladný výzkum už běží; rychlé dotazy můžete zadávat dál.",
    ),
    (
        "A quick question or automatic routing is already running; wait or stop it first",
        "Rychlý dotaz nebo výběr režimu už běží; počkejte nebo ho zastavte.",
    ),
    (
        "Auto selected thorough research, but that lane is busy; choose Quick or wait for the running research",
        "Auto zvolilo důkladný výzkum, ale ten už běží. Zvolte Rychle nebo počkejte na jeho dokončení.",
    ),
    (
        "The saved research meeting changed; start a new question",
        "Schůzka uloženého výzkumu se změnila; začněte novým dotazem.",
    ),
    (
        "This saved research task is unavailable",
        "Tento uložený výzkum není dostupný.",
    ),
    (
        "Research connections or source scope changed; start a new question instead of reusing this checkpoint",
        "Změnila se připojení nebo rozsah zdrojů výzkumu; místo pokračování položte nový dotaz.",
    ),
    (
        "Connect the calendar account before starting resumable research",
        "Před spuštěním výzkumu s pokračováním připojte účet kalendáře.",
    ),
    (
        "Eight interrupted research tasks are saved; finish a saved task before starting another",
        "Je uloženo osm přerušených výzkumů; před novým některý dokončete nebo zahoďte.",
    ),
    ("Demo", "Demo"),
    (
        "Offline demo - fictional calendar",
        "Offline demo - smyšlený kalendář",
    ),
    (
        "Sample Insights - not generated by AI",
        "Ukázkový přehled - bez generování AI",
    ),
    ("Meeting actions", "Akce schůzky"),
    ("Meeting action", "Změna schůzky"),
    ("Check calendar result", "Ověřit výsledek změny"),
    (
        "Calendar verification could not be queued; no write was repeated",
        "Ověření změny se nepodařilo zařadit do fronty. Žádný zápis se neopakoval.",
    ),
    (
        "Calendar verification timed out; no write was repeated",
        "Ověření změny překročilo časový limit. Žádný zápis se neopakoval.",
    ),
    (
        "Calendar verification is unavailable or busy; no write was repeated",
        "Ověření změny teď není dostupné nebo právě probíhá jiná operace. Žádný zápis se neopakoval.",
    ),
    (
        "Checking the previous calendar result; no write will be repeated",
        "Ověřuji výsledek předchozí změny. Žádný zápis se nebude opakovat.",
    ),
    (
        "Read exact calendar approval failed",
        "Nepodařilo se načíst přesné schválení změny.",
    ),
    (
        "Saved calendar approval is invalid",
        "Uložené schválení změny není platné.",
    ),
    (
        "Saved manual calendar action has no exact change",
        "Uložená akce schůzky neobsahuje přesnou změnu.",
    ),
    (
        "Calendar removal identity did not match its approval",
        "Smazaná událost neodpovídá schválené změně.",
    ),
    (
        "Confirmed calendar approval disappeared",
        "Uložené schválení změny kalendáře není dostupné.",
    ),
    ("Confirm meeting action", "Potvrzení změny schůzky"),
    ("Change category", "Změnit kategorii"),
    ("Remove category", "Odebrat kategorii"),
    ("Accept invitation", "Přijmout pozvánku"),
    ("Tentatively accept", "Přijmout nezávazně"),
    ("Decline invitation", "Odmítnout pozvánku"),
    ("Cancel organized meeting", "Zrušit schůzku pro všechny"),
    ("Delete own appointment", "Smazat vlastní událost"),
    ("Account", "Účet"),
    ("Organizer", "Organizátor"),
    ("Shift+F10: meeting actions", "Shift+F10: akce schůzky"),
    ("Comment (optional)", "Komentář (nepovinný)"),
    ("Delete event", "Smazat událost"),
    ("Delete", "Smazat"),
    ("Back", "Zpět"),
    ("Ask AI", "Zeptat se AI"),
    ("Hide Ask", "Skrýt"),
    ("Stop", "Zastavit"),
    ("Dock Ask", "Zadokovat"),
    ("Detach Ask", "Samostatné okno"),
    ("Answer ready", "Odpověď je připravená"),
    ("Meeting context", "Ke schůzce"),
    ("Clear context", "Odebrat kontext"),
    ("General question", "Obecný dotaz"),
    ("Ask is updating...", "Ask se aktualizuje…"),
    (
        "Ask is updating. Try changing context again.",
        "Ask se aktualizuje. Zkus kontext změnit znovu.",
    ),
    ("Use selected meeting", "K vybrané schůzce"),
    (
        "Ask a question or type / for commands",
        "Napiš dotaz nebo / pro příkazy",
    ),
    ("What is on my calendar today", "Co mám dnes v kalendáři"),
    (
        "Important meetings this week",
        "Důležité schůzky tento týden",
    ),
    (
        "Prepare for the contextual meeting",
        "Příprava na schůzku v kontextu",
    ),
    ("Explain calendar conflicts", "Vysvětlit souběhy schůzek"),
    (
        "Find a meeting, message or material",
        "Najít schůzku, zprávu nebo podklady",
    ),
    (
        "Missing travel time: next 14 days, results in Messages",
        "Doplnit přejezdy: příštích 14 dní, návrhy do zvonečku",
    ),
    (
        "Review categories: next 14 days, results in Messages",
        "Kontrola kategorií: příštích 14 dní, návrhy do zvonečku",
    ),
    (
        "Use /travel or /categories without arguments; each checks the next 14 days.",
        "Použij /travel nebo /categories bez dalších parametrů; oba kontrolují příštích 14 dní.",
    ),
    (
        "Travel review queued for the next 14 days. Suggestions will appear in Messages; calendar changes still need approval.",
        "Kontrola přejezdů na příštích 14 dní byla předána na pozadí. Návrhy najdeš ve zvonečku; změny kalendáře stále vyžadují schválení.",
    ),
    (
        "Category review queued for the next 14 days. Suggestions will appear in Messages; calendar changes still need approval.",
        "Kontrola kategorií na příštích 14 dní byla předána na pozadí. Návrhy najdeš ve zvonečku; změny kalendáře stále vyžadují schválení.",
    ),
    (
        "Connect AI and WorkIQ before starting a calendar review.",
        "Před kontrolou kalendáře připoj AI a WorkIQ.",
    ),
    (
        "Resolve the previous preparation failure before starting a calendar review.",
        "Před kontrolou kalendáře vyřeš předchozí chybu přípravy.",
    ),
    (
        "Refresh the next 14 days of the calendar before starting this review.",
        "Před kontrolou nejdřív načti kalendář na příštích 14 dní.",
    ),
    (
        "Some meetings have failed or interrupted research; inspect and retry them before this review.",
        "U některých schůzek zjišťování selhalo nebo bylo přerušeno. Před kontrolou je zkontroluj a zopakuj.",
    ),
    (
        "Calendar account changed before the review; nothing was queued.",
        "Před kontrolou se změnil účet kalendáře; nic nebylo zařazeno do fronty.",
    ),
    (
        "Calendar review stopped after the account or AI configuration changed. Run the command again.",
        "Kontrola kalendáře byla zastavena po změně účtu nebo nastavení AI. Spusť příkaz znovu.",
    ),
    (
        "Calendar reviews need the live worker; previews never submit requests.",
        "Kontroly kalendáře vyžadují připojené služby; ukázkový režim žádné požadavky neodesílá.",
    ),
    (
        "Unknown Ask command. Choose a command below or ask without a leading slash.",
        "Neznámý příkaz. Vyber příkaz z nabídky nebo napiš dotaz bez úvodního lomítka.",
    ),
    (
        "Choose a meeting context before using /prepare.",
        "Pro /prepare nejdřív vyber kontext schůzky.",
    ),
    (
        "After /find, enter what you want to find.",
        "Za /find napiš, co chceš najít.",
    ),
    (
        "Stop the current question before changing its meeting context.",
        "Před změnou kontextu schůzky nejdřív zastav aktuální dotaz.",
    ),
    (
        "Meeting context changed. Review the question before submitting it.",
        "Kontext schůzky se změnil. Před odesláním zkontroluj dotaz.",
    ),
    (
        "The Ask window is unavailable. Restart Tomlook.",
        "Okno Ask není dostupné. Restartuj Tomlook.",
    ),
    (
        "The Ask window state is unavailable. Restart Tomlook.",
        "Stav okna Ask není dostupný. Restartuj Tomlook.",
    ),
    (
        "Separate Ask windows are not supported by this renderer. Ask remains docked.",
        "Samostatná okna nejsou podporována. Ask zůstává v pravém panelu.",
    ),
    (
        "The Ask action queue is full. Nothing was submitted; try again.",
        "Fronta akcí Ask je plná. Nic nebylo odesláno; zkus to znovu.",
    ),
    (
        "Summarize my calendar for this local date, with important meetings and preparation needs:",
        "Shrň můj kalendář pro toto místní datum, včetně důležitých schůzek a potřebné přípravy:",
    ),
    (
        "Summarize important meetings and preparation needs for this local calendar week:",
        "Shrň důležité schůzky a potřebnou přípravu pro tento týden místního kalendáře:",
    ),
    (
        "Explain how to prepare for the contextual meeting using saved Insights and retrieved evidence. Distinguish verified needs, reusable material and missing information.",
        "Vysvětli přípravu na schůzku v kontextu pomocí uložených Insights a získaných podkladů. Rozliš ověřené potřeby, znovupoužitelné materiály a chybějící informace.",
    ),
    (
        "Find conflicts in my calendar for these local dates. Name both meetings and their full local time ranges, and explain practical resolution options without changing the calendar:",
        "Najdi souběhy v mém kalendáři pro tato místní data. Uveď obě schůzky a jejich celé místní časové rozsahy. Vysvětli možnosti řešení bez změny kalendáře:",
    ),
    (
        "Find the requested meeting, message or preparation material in authorized workplace sources. Cite retrieved sources and distinguish no matches from unavailable access.",
        "Najdi požadovanou schůzku, zprávu nebo podklady v oprávněných pracovních zdrojích. Uveď získané zdroje a rozliš nenalezené výsledky od nedostupného přístupu.",
    ),
    ("More options", "Další možnosti"),
    ("Copy answer", "Kopírovat odpověď"),
    ("Thinking...", "Přemýšlím…"),
    ("This occurrence only", "Jen tento výskyt"),
    ("Notifies all attendees.", "Oznámí se všem účastníkům."),
    (
        "Send response to organizer",
        "Odeslat odpověď organizátorovi",
    ),
    (
        "Confirmation sends this response and comment.",
        "Potvrzením odešlete tuto odpověď a komentář.",
    ),
    (
        "No message will be sent to the organizer.",
        "Organizátorovi se neodešle žádná zpráva.",
    ),
    (
        "Only this occurrence will change, never the whole series.",
        "Změní se jen tato událost nebo výskyt, nikdy celá série.",
    ),
    (
        "Cancels your meeting and notifies all attendees.",
        "Zruší vaši schůzku a odešle oznámení všem účastníkům.",
    ),
    (
        "Removes your own appointment. There are no attendees to notify.",
        "Smaže vaši vlastní událost. Nemá žádné účastníky, kterým by se odesílalo oznámení.",
    ),
    (
        "All existing categories will be removed.",
        "Odeberou se všechny dosavadní kategorie.",
    ),
    (
        "Meeting or account changed; close this dialog and review the current meeting.",
        "Schůzka nebo účet se změnily. Zavřete dialog a zkontrolujte aktuální schůzku.",
    ),
    (
        "Refresh the calendar first; actions require one verified occurrence, not a whole series",
        "Nejdřív obnovte kalendář. Akce vyžadují ověřenou jednotlivou událost nebo výskyt, ne celou sérii.",
    ),
    (
        "Cancelled meetings cannot be changed",
        "Zrušené schůzky nelze měnit.",
    ),
    (
        "Comment exceeds its 4096-byte limit or contains unsupported control characters",
        "Komentář je příliš dlouhý (limit 4096 bajtů) nebo obsahuje nepodporované řídicí znaky.",
    ),
    (
        "A comment requires sending the response",
        "Komentář lze přidat jen k odesílané odpovědi.",
    ),
    (
        "Meeting action does not match your organizer or attendee role",
        "Tuto akci nelze provést v roli, kterou u schůzky máte.",
    ),
    (
        "Calendar provider has no verified meeting action capability",
        "Poskytovatel nemá ověřenou podporu této akce. Nic nebylo odesláno.",
    ),
    (
        "Review the previous calendar operation before confirming another change",
        "Před další změnou zkontrolujte výsledek předchozí operace s kalendářem.",
    ),
    (
        "The meeting already has no category; no write was submitted",
        "Schůzka už nemá kategorii. Žádný zápis nebyl odeslán.",
    ),
    (
        "The meeting already has this response; no write was submitted",
        "Schůzka už má tuto odpověď. Nic nebylo odesláno.",
    ),
    ("Category", "Kategorie"),
    (
        "Calendar coverage is incomplete; no write was submitted",
        "Kalendář není načtený pro celý navržený čas. Žádný zápis nebyl odeslán.",
    ),
    ("Recommendations", "Doporučení"),
    ("Diagnostics", "Diagnostika"),
    ("Meeting", "Schůzka"),
    ("Make time to prepare", "Vyhradit čas na přípravu"),
    (
        "Reserve travel before meeting",
        "Přidat přesun před schůzkou",
    ),
    ("Reserve travel after meeting", "Přidat přesun po schůzce"),
    ("Travel before meeting", "Přesun před schůzkou"),
    ("Travel after meeting", "Přesun po schůzce"),
    ("Use category", "Použít kategorii"),
    ("Add to calendar", "Přidat do kalendáře"),
    ("Skip recommendation", "Přeskočit"),
    ("Details and checks", "Podrobnosti"),
    ("Needs review", "Ke kontrole"),
    ("Preparation", "Příprava"),
    ("Advance preparation", "Příprava v předstihu"),
    ("Prepare in advance", "Připravit v předstihu"),
    ("To category", "Na kategorii"),
    ("Conflicts with", "Překrývá se s"),
    ("Who to ask", "Koho oslovit"),
    ("Same email domain", "Stejná e-mailová doména"),
    ("Participant", "Účastník"),
    (
        "Ask for the agenda, materials and your expected role.",
        "Požádej o agendu, podklady a svou očekávanou roli.",
    ),
    ("Overlap", "Překryv"),
    (
        "Confirmed calendar request is invalid; no write was submitted",
        "Potvrzený požadavek není platný. Zápis nebyl odeslán.",
    ),
    (
        "No overlap in the current confirmed calendar.",
        "V aktuálním kalendáři není překryv s potvrzenou událostí.",
    ),
    ("Alt+Up / Alt+Down", "Alt+nahoru / Alt+dolů"),
    (
        "Read dismissed recommendations failed",
        "Nepodařilo se načíst přeskočená doporučení.",
    ),
    (
        "Saved dismissed recommendations are invalid",
        "Uložená přeskočená doporučení jsou neplatná.",
    ),
    (
        "Dismissed recommendations reached their safety limit",
        "Přeskočená doporučení dosáhla bezpečnostního limitu; historie nebyla odstraněna.",
    ),
    (
        "Encode dismissed recommendations failed",
        "Nepodařilo se připravit přeskočená doporučení k uložení.",
    ),
    (
        "Persist dismissed recommendations failed",
        "Nepodařilo se uložit přeskočená doporučení.",
    ),
    ("Travel buffer", "Rezerva na přejezd"),
    ("Missing materials", "Chybějící podklady"),
    ("Priority conflict", "Souběh důležitých schůzek"),
    ("Follow-up", "Připomenutí po schůzce"),
    ("Meeting transition", "Přechod mezi schůzkami"),
    ("Why?", "Proč?"),
    ("None", "Žádná"),
    ("Previous", "Předchozí"),
    ("Next", "Další"),
    ("Confirm calendar change", "Potvrdit změnu kalendáře"),
    ("Open Insights", "Otevřít přehled"),
    ("Draft follow-up", "Připravit navazující koncept"),
    (
        "Prepare a local follow-up draft for this meeting using saved Insights and retrieved evidence. Separate verified outcomes from pre-meeting plans; use placeholders for unknown decisions, owners and dates. Do not send any message, invite attendees or modify the calendar.",
        "Připrav lokální koncept navazující zprávy k této schůzce podle uloženého přehledu a získaných podkladů. Odděl ověřené výsledky od plánů před schůzkou; pro neznámá rozhodnutí, odpovědné osoby a termíny použij zástupná místa. Nic neodesílej, nezvi účastníky ani neměň kalendář.",
    ),
    (
        "An unsent question is already saved for this meeting; it was preserved. Review it before asking for a follow-up draft.",
        "U této schůzky už máš rozepsaný dotaz. Zůstal zachovaný; před přípravou navazujícího konceptu ho zkontroluj.",
    ),
    (
        "Recommendation changed or is no longer available; refresh Insights before preparing a draft.",
        "Doporučení se změnilo nebo už není dostupné. Před přípravou konceptu obnov přehled schůzky.",
    ),
    (
        "Meeting is no longer in the current calendar.",
        "Schůzka už není v aktuálním kalendáři.",
    ),
    (
        "Replaces all previous categories with this single category.",
        "Nahradí všechny původní kategorie touto jedinou.",
    ),
    (
        "Suggested slots use weekdays, 08:00-18:00 local time.",
        "Návrhy používají pracovní dny od 8 do 18 hodin místního času.",
    ),
    (
        "Free in the confirmed calendar; actual availability is not verified.",
        "Mezi potvrzenými schůzkami je volno. Skutečná dostupnost není ověřená.",
    ),
    (
        "Travel duration is an estimate; live traffic has not been verified.",
        "Doba přejezdu je odhad. Aktuální provoz není ověřený.",
    ),
    (
        "No free interval fits the proposed buffer; adjust the calendar first",
        "Pro navrženou rezervu není volný úsek. Nejdřív upravte kalendář.",
    ),
    (
        "The confirmed buffer must match its proposed duration and side of the meeting",
        "Potvrzený blok musí mít navrženou délku a být na správné straně schůzky.",
    ),
    (
        "Offsite travel recommendations exceed their safety limit",
        "Počet doporučení na přesun překročil bezpečnostní limit.",
    ),
    (
        "Recommendation expired after a calendar change",
        "Po změně kalendáře už doporučení není aktuální.",
    ),
    (
        "Calendar change verified by read-back",
        "Změna kalendáře byla ověřená opětovným načtením.",
    ),
    (
        "Confirmed calendar change is running",
        "Provádí se potvrzená změna kalendáře.",
    ),
    (
        "Calendar change was not queued; nothing was submitted",
        "Změna se nedostala do fronty. Do kalendáře nebyl odeslán žádný zápis.",
    ),
    (
        "Calendar writes are unavailable or busy; nothing was submitted",
        "Zápis do kalendáře není dostupný nebo právě probíhá jiný zápis. Nic nebylo odesláno.",
    ),
    (
        "Tomlook is stopping; no calendar write was submitted",
        "Tomlook se ukončuje. Do kalendáře nebyl odeslán žádný zápis.",
    ),
    (
        "Connect WorkIQ before confirming a calendar change",
        "Před potvrzením změny kalendáře připojte WorkIQ.",
    ),
    (
        "Calendar approval was already consumed; inspect the previous outcome",
        "Toto potvrzení už bylo použité. Zkontrolujte předchozí výsledek.",
    ),
    (
        "The proposed interval or action changed; review the current recommendation before confirming",
        "Navržený čas nebo změna už nejsou stejné. Před potvrzením si prohlédněte aktuální doporučení.",
    ),
    (
        "Meeting changed after the recommendation; no calendar write was submitted",
        "Schůzka se od vytvoření doporučení změnila. Žádný zápis nebyl odeslán.",
    ),
    (
        "The meeting already has the proposed category; no write was submitted",
        "Schůzka už má navrženou kategorii. Žádný zápis nebyl odeslán.",
    ),
    (
        "The proposed interval is no longer free; no calendar write was submitted",
        "Navržený čas už není volný. Žádný zápis nebyl odeslán.",
    ),
    (
        "WorkIQ denied this calendar change. Your tenant may block mutations; an administrator must enable calendar writes. Nothing was changed by this request.",
        "WorkIQ tuto změnu odmítl. Zápisy může blokovat pravidlo organizace; povolit je musí správce. Tento požadavek nic nezměnil.",
    ),
    (
        "Calendar write outcome is uncertain. Inspect Outlook before proposing another change; this operation will not be replayed automatically.",
        "Výsledek zápisu do kalendáře není jistý. Před další změnou zkontrolujte Outlook. Tento zápis se automaticky neopakuje.",
    ),
    (
        "Calendar write was returned but read-back did not verify the exact change. Inspect Outlook; the operation will not be replayed automatically.",
        "Zápis dostal odpověď, ale opětovné načtení nepotvrdilo přesnou změnu. Zkontrolujte Outlook. Zápis se automaticky neopakuje.",
    ),
    (
        "WorkIQ has no compatible calendar write tool; read-only research remains available",
        "WorkIQ nenabízí kompatibilní nástroj pro zápis do kalendáře. Čtení a hledání souvislostí zůstává dostupné.",
    ),
    (
        "Calendar schema discovery timed out",
        "Vypršel čas pro ověření rozhraní kalendáře.",
    ),
    (
        "Calendar schema discovery failed; no write was submitted",
        "Rozhraní kalendáře se nepodařilo ověřit. Žádný zápis nebyl odeslán.",
    ),
    (
        "Calendar provider did not return a compatible JSON schema; no write was submitted",
        "Poskytovatel nevrátil kompatibilní popis rozhraní kalendáře. Žádný zápis nebyl odeslán.",
    ),
    (
        "Recommendation inbox reached its safety limit",
        "Schránka doporučení dosáhla bezpečnostního limitu.",
    ),
    ("AI usage", "Spotřeba AI"),
    ("Daily AI request limit", "Denní limit AI požadavků"),
    ("Requests", "Požadavky"),
    ("Input tokens", "Vstupní tokeny"),
    ("Cache read tokens", "Tokeny načtené z cache"),
    ("Cache write tokens", "Tokeny uložené do cache"),
    ("Output tokens", "Výstupní tokeny"),
    ("Unavailable", "Není k dispozici"),
    (
        "Requests with reported usage",
        "Požadavky s nahlášenou spotřebou",
    ),
    (
        "Measured SDK tokens only; missing usage is not zero or a currency estimate.",
        "Pouze tokeny nahlášené SDK. Chybějící údaj neznamená nulu ani odhad ceny.",
    ),
    (
        "Usage accounting is loading.",
        "Načítá se přehled spotřeby.",
    ),
    (
        "Retrying saved sign-in...",
        "Obnovuje se uložené připojení...",
    ),
    (
        "Unconfirmed meeting: prepare explicitly when needed",
        "Nepotvrzená schůzka: přípravu spusťte podle potřeby",
    ),
    (
        "Daily AI request limit reached; queued research is deferred until tomorrow.",
        "Denní limit AI požadavků byl vyčerpán. Příprava ve frontě bude pokračovat zítra.",
    ),
    (
        "AI diagnostics or usage could not be saved completely; inspect the application log before relying on totals.",
        "Diagnostiku nebo spotřebu AI se nepodařilo uložit úplně. Před použitím součtů zkontrolujte protokol aplikace.",
    ),
    ("Today", "Dnes"),
    ("Task note", "Poznámka k úkolu"),
    ("Email draft", "Koncept e-mailu"),
    ("Calendar suggestion", "Návrh do kalendáře"),
    ("Target", "Cíl"),
    ("Body", "Text"),
    (
        "Workplace search summary",
        "Souhrn hledání v pracovních zdrojích",
    ),
    ("Message read", "Původní zpráva"),
    ("Meeting read", "Původní schůzka"),
    ("Mail search results", "Výsledky hledání v poště"),
    ("Calendar results", "Výsledky z kalendáře"),
    ("Mail and meetings", "Pošta a schůzky"),
    ("Customer documents", "Dokumenty zákazníka"),
    ("Local demos and repositories", "Místní dema a repozitáře"),
    ("Local document", "Místní dokument"),
    ("Checking saved sign-in...", "Ověřuji uložené přihlášení…"),
    (
        "Sign-in expired; sign in again",
        "Přihlášení vypršelo. Přihlaste se znovu.",
    ),
    ("Indexing local sources...", "Načítám místní zdroje…"),
    (
        "No local sources configured; cloud research uses WorkIQ",
        "Místní zdroje nejsou nastavené. Podklady v cloudu se hledají přes WorkIQ.",
    ),
    (
        "Preview does not connect to providers",
        "Náhled se nepřipojuje ke službám",
    ),
    ("Connection is already busy", "Připojení už pracuje"),
    (
        "Saved sign-in removal is already in progress",
        "Uložené přihlášení se už maže",
    ),
    (
        "AI is already connected or connecting.",
        "AI je už připojená nebo se připojuje.",
    ),
    (
        "Starting Copilot device sign-in; credentials stay encrypted in Tomlook...",
        "Spouštím přihlášení k Copilotu pomocí kódu. Přihlašovací údaje budou v Tomlooku šifrované…",
    ),
    (
        "WorkIQ is connected or busy; disconnect before changing accounts.",
        "WorkIQ je připojené nebo pracuje. Před změnou účtu jej odpojte.",
    ),
    ("Connecting WorkIQ...", "Připojuji WorkIQ…"),
    (
        "Removing WorkIQ sign-in; cached calendar retained.",
        "Mažu přihlášení k WorkIQ. Uložený kalendář zůstane zachovaný.",
    ),
    (
        "Sign in to WorkIQ before refreshing the calendar.",
        "Před aktualizací kalendáře se přihlaste k WorkIQ.",
    ),
    ("Connecting isolated SDK...", "Připojuji izolované SDK…"),
    (
        "Isolated SDK is already connected",
        "Izolované SDK je už připojené",
    ),
    (
        "Connecting isolated SDK on its own worker...",
        "Připojuji izolované SDK na pozadí…",
    ),
    (
        "Disconnected; saved sign-in removed",
        "Odpojeno. Uložené přihlášení bylo smazáno.",
    ),
    (
        "WorkIQ connection cancelled; no new reads are admitted.",
        "Připojení WorkIQ bylo zrušeno. Další načítání se nespustí.",
    ),
    (
        "WorkIQ refresh cancelled after disconnect; no calendar was published.",
        "Po odpojení byla aktualizace WorkIQ zrušena. Kalendář se nezměnil.",
    ),
    (
        "WorkIQ disconnected. Cached calendar retained; work credentials released.",
        "WorkIQ je odpojené. Uložený kalendář zůstává dostupný a přihlašovací údaje byly uvolněny.",
    ),
    (
        "AI sign-in completed.",
        "Přihlášení k AI proběhlo úspěšně.",
    ),
    ("Stopping...", "Ukončuji…"),
    (
        "WorkIQ disconnected; sign in again",
        "WorkIQ je odpojené. Přihlaste se znovu.",
    ),
    (
        "WorkIQ connection expired or failed during the answer; sign in again.",
        "Během zpracování odpovědi připojení WorkIQ vypršelo nebo selhalo. Přihlaste se znovu.",
    ),
    (
        "Meeting removed, cancelled, elapsed or outside preparation horizon",
        "Schůzka zmizela, byla zrušena, skončila nebo je mimo období přípravy",
    ),
    (
        "Meeting or AI configuration changed; inspect the previous outcome before explicit retry",
        "Schůzka nebo nastavení AI se změnily. Před opakováním zkontrolujte předchozí výsledek.",
    ),
    (
        "Meeting or AI configuration changed",
        "Schůzka nebo nastavení AI se změnily",
    ),
    (
        "Refreshing upcoming meeting context",
        "Obnovuji souvislosti k nadcházející schůzce",
    ),
    (
        "Calendar coverage is unverified; no automatic analysis",
        "Kalendář v tomto období ještě není ověřený. Automatická analýza se nespustí.",
    ),
    (
        "Waiting for an execution slot",
        "Čeká na volné místo pro zpracování",
    ),
    (
        "Deferred: bounded queue is full",
        "Odloženo: fronta je plná",
    ),
    (
        "Refresh this meeting's calendar before preparing it.",
        "Před zjištěním souvislostí aktualizujte kalendář této schůzky.",
    ),
    (
        "This meeting is already being prepared.",
        "Souvislosti k této schůzce se už zjišťují.",
    ),
    (
        "Preparation could not be queued.",
        "Zjištění souvislostí se nepodařilo zařadit do fronty.",
    ),
    (
        "Yielded to an explicit meeting preparation",
        "Uvolněno pro ručně vyžádané zjištění souvislostí",
    ),
    ("AI is not connected", "AI není připojená"),
    (
        "Execution capacity is reserved or busy",
        "Zpracování je obsazené nebo vyhrazené",
    ),
    (
        "Saved result validated; citations are not independently verified",
        "Uložený výsledek prošel kontrolou formátu. Odkazy nejsou nezávisle ověřené.",
    ),
    ("Switch language", "Přepnout jazyk"),
    ("Refresh", "Obnovit"),
    ("Tomlook Focus logo", "Logo Tomlook Focus"),
    ("Ask about this meeting", "Zeptat se na tuto schůzku"),
    (
        "Translated saved Insights; no new research was performed.",
        "Přeložený uložený přehled. Nové podklady se nedohledávaly.",
    ),
    (
        "Czech version is not saved yet. Research now can create both languages.",
        "Česká verze zatím není uložená. Zjištění souvislostí vytvoří obě jazykové verze.",
    ),
    ("Sources not indexed yet", "Zdroje zatím nejsou načtené"),
    (
        "WorkIQ authentication has not been tested",
        "Přihlášení k WorkIQ zatím nebylo ověřené",
    ),
    (
        "Local queue is full. Nothing was submitted; try again.",
        "Místní fronta je plná. Nic se neodeslalo, zkuste to znovu.",
    ),
    (
        "The storage worker is unavailable. Restart Tomlook.",
        "Ukládání není dostupné. Restartujte Tomlook.",
    ),
    (
        "AI shutdown coordinator is unavailable",
        "Řízené ukončení AI není dostupné",
    ),
    (
        "The meeting changed or disappeared. The old result was invalidated and cancellation requested; review the updated context before asking again.",
        "Schůzka se změnila nebo zmizela. Předchozí odpověď už není platná a bylo vyžádáno zrušení dotazu. Než se znovu zeptáte, zkontrolujte souvislosti.",
    ),
    (
        "Company icon exceeds its display limit",
        "Ikona společnosti překračuje limit pro zobrazení",
    ),
    (
        "No icon found on the company's own website",
        "Na webu společnosti nebyla nalezena ikona",
    ),
    (
        "Photo exceeds its display limit",
        "Fotografie překračuje limit pro zobrazení",
    ),
    (
        "Photo lookup could not be admitted",
        "Načtení fotografie se nepodařilo zařadit do fronty",
    ),
    (
        "AI response exceeded the local-proposal limit",
        "Odpověď AI obsahuje příliš mnoho místních návrhů",
    ),
    (
        "AI answer exceeded the 64 KiB response limit; no answer was published.",
        "Odpověď AI překročila limit 64 KiB a nebyla zobrazena.",
    ),
    ("AI stopped", "AI byla zastavena"),
    ("Messages", "Zprávy"),
    ("No messages.", "Žádné zprávy."),
    ("Review interrupted jobs", "Zkontrolovat přerušené úlohy"),
    (
        "Renewing saved GitHub sign-in...",
        "Obnovuji přihlášení ke GitHubu…",
    ),
    (
        "AI executor stopped before an outcome was recorded; inspect saved evidence before explicit retry",
        "AI skončila bez uloženého výsledku. Zkontrolujte podklady; tuto úlohu spusťte znovu ručně.",
    ),
    (
        "Saved GitHub sign-in has no refresh token; sign in once to enable automatic renewal",
        "Starší přihlášení ke GitHubu neobsahuje obnovovací token. Přihlaste se jednou znovu, další obnova už bude automatická.",
    ),
    (
        "Saved GitHub refresh credentials are incomplete; sign in again",
        "Obnovovací údaje GitHubu nejsou úplné. Přihlaste se znovu.",
    ),
    (
        "GitHub token renewal was rejected; sign in again",
        "GitHub odmítl obnovu přihlášení. Přihlaste se znovu.",
    ),
    (
        "Saved GitHub sign-in cannot be renewed; sign in again",
        "Uložené přihlášení ke GitHubu nelze obnovit. Přihlaste se znovu.",
    ),
    (
        "GitHub refresh token expired; sign in again",
        "Platnost obnovovacího tokenu GitHubu skončila. Přihlaste se znovu.",
    ),
    (
        "GitHub token renewal could not reach the provider; retrying shortly",
        "GitHub není pro obnovu přihlášení dostupný. Obnovu zkusíme znovu.",
    ),
    (
        "GitHub token renewal returned no refresh token; sign in again",
        "GitHub při obnově nevrátil obnovovací token. Přihlaste se znovu.",
    ),
    (
        "GitHub returned an invalid token lifetime",
        "GitHub vrátil neplatnou dobu platnosti tokenu.",
    ),
    (
        "Preparation outcome delivery timed out; inspect the interrupted ledger",
        "Předání výsledku přehledu překročilo časový limit. Zkontrolujte přerušenou úlohu.",
    ),
    (
        "Preparation outcome could not reach storage; inspect the interrupted ledger",
        "Výsledek přehledu se nepodařilo předat úložišti. Zkontrolujte přerušenou úlohu.",
    ),
    (
        "Connector configuration could not reach storage; restart before new research",
        "Nastavení připojení se nepodařilo předat úložišti. Před dalším výzkumem restartujte Tomlook.",
    ),
    (
        "AI is stopping; the previous result was discarded",
        "AI se ukončuje; nedokončený výsledek nebyl zveřejněn",
    ),
    (
        "Cancelled before the question was sent",
        "Zrušeno před odesláním dotazu",
    ),
    (
        "AI stopped without a completed answer; provider outcome may be unknown",
        "AI skončila bez úplné odpovědi. Výsledek u poskytovatele nemusí být známý.",
    ),
    (
        "AI command queue is full; nothing was submitted. Try again.",
        "Fronta AI je plná. Nic se neodeslalo, zkuste to znovu.",
    ),
    (
        "The AI worker is unavailable. Restart Tomlook.",
        "AI není dostupná. Restartujte Tomlook.",
    ),
    (
        "Cancellation requested; partial output is not a completed answer. Provider outcome may be unknown.",
        "Bylo vyžádáno zrušení dotazu. Částečný výstup není hotová odpověď a výsledek u poskytovatele nemusí být známý.",
    ),
    (
        "AI cancellation could not be submitted. Its result will be discarded, but the provider request may continue until timeout or application exit.",
        "Zrušení dotazu se nepodařilo odeslat. Odpověď bude zahozena, ale požadavek u poskytovatele může pokračovat až do vypršení limitu nebo ukončení aplikace.",
    ),
    (
        "Meeting context changed. The previous answer was invalidated; review your draft before asking again.",
        "Souvislosti schůzky se změnily. Předchozí odpověď už není platná. Než se znovu zeptáte, zkontrolujte rozepsaný dotaz.",
    ),
    (
        "Assistant memory holds 32 contexts. Return to an existing context or restart Tomlook to clear memory. No new question can be submitted for the requested context.",
        "Asistent má v paměti 32 kontextů. Vraťte se k některému z nich nebo restartujte Tomlook. Pro nový kontext zatím nelze odeslat dotaz.",
    ),
    (
        "Use a nonempty question of at most 8,000 bytes.",
        "Napište dotaz o délce nejvýše 8 000 bajtů.",
    ),
    (
        "Assistant request identity limit reached. No question was submitted; restart Tomlook.",
        "Byl dosažen limit dotazů asistenta. Nic se neodeslalo. Restartujte Tomlook.",
    ),
    (
        "Resolve the assistant context warning before keeping a draft; nothing was kept",
        "Než uložíte koncept, vyřešte upozornění na kontext asistenta. Nic nebylo uloženo.",
    ),
    (
        "Eight local drafts already occupy this context; discard one before keeping another",
        "V tomto kontextu už je osm konceptů. Před uložením dalšího některý zahoďte.",
    ),
    (
        "This proposal is already a local draft; nothing was duplicated",
        "Tento návrh už je uložený jako koncept. Další kopie nevznikla.",
    ),
    (
        "Local draft identity limit reached; nothing was added. Restart Tomlook to clear memory.",
        "Byl dosažen limit místních konceptů. Nic nepřibylo. Paměť uvolníte restartem Tomlooku.",
    ),
    (
        "Resolve the assistant context warning before adding a draft; nothing was added",
        "Než přidáte koncept, vyřešte upozornění na kontext asistenta. Nic nepřibylo.",
    ),
    (
        "Eight local drafts already occupy this context; discard one before adding another",
        "V tomto kontextu už je osm konceptů. Před přidáním dalšího některý zahoďte.",
    ),
    (
        "Calendar view has no first day",
        "Zobrazení kalendáře nemá první den",
    ),
    (
        "Calendar view has no last day",
        "Zobrazení kalendáře nemá poslední den",
    ),
    (
        "Calendar refresh queued...",
        "Aktualizace kalendáře čeká ve frontě…",
    ),
    (
        "AI device sign-in queued...",
        "Přihlášení k AI čeká ve frontě…",
    ),
    ("Removing saved sign-in...", "Mažu uložené přihlášení…"),
    (
        "WorkIQ connection queued...",
        "Připojení WorkIQ čeká ve frontě…",
    ),
    (
        "Removing saved WorkIQ sign-in...",
        "Mažu uložené přihlášení k WorkIQ…",
    ),
    (
        "Use up to eight specific document folders and four absolute repository collection paths.",
        "Zadejte nejvýše osm konkrétních složek s dokumenty a čtyři absolutní cesty ke složkám s repozitáři.",
    ),
    ("Day", "Den"),
    ("Work week", "Pracovní týden"),
    ("Week", "Týden"),
    ("Month", "Měsíc"),
    ("Monday", "Pondělí"),
    ("Tuesday", "Úterý"),
    ("Wednesday", "Středa"),
    ("Thursday", "Čtvrtek"),
    ("Friday", "Pátek"),
    ("Saturday", "Sobota"),
    ("Sunday", "Neděle"),
    ("Commands", "Příkazy"),
    ("Accent", "Barva"),
    ("Blue", "Modrá"),
    ("Red", "Červená"),
    ("Green", "Zelená"),
    ("Yellow", "Žlutá"),
    ("Light", "Světlý"),
    ("Dark", "Tmavý"),
    ("Ask", "Zeptat se"),
    ("Clear", "Vymazat"),
    ("Search meetings", "Hledat schůzky"),
    ("Search commands...", "Hledat příkazy…"),
    ("No matching commands", "Žádné odpovídající příkazy"),
    ("Previous period - Alt+Left", "Předchozí období – Alt+vlevo"),
    ("Next period - Alt+Right", "Další období – Alt+vpravo"),
    ("Show this work week", "Zobrazit tento pracovní týden"),
    ("Hide unconfirmed", "Skrýt nepotvrzené"),
    ("Unconfirmed", "Nepotvrzeno"),
    ("Confirmed", "Potvrzeno"),
    ("Tentative", "Nezávazně"),
    ("Following", "Sledováno"),
    ("Declined", "Odmítnuto"),
    ("All day", "Celý den"),
    ("Not synced", "Nenačteno"),
    ("CATEGORIES", "KATEGORIE"),
    ("All meetings", "Všechny schůzky"),
    ("KEYBOARD", "KLÁVESOVÉ ZKRATKY"),
    ("Ctrl+F   Search", "Ctrl+F   Hledání"),
    ("Ctrl+Shift+P   Commands", "Ctrl+Shift+P   Příkazy"),
    ("Ctrl+Space   Ask", "Ctrl+Space   Dotaz"),
    ("Alt+Left / Right   Period", "Alt+vlevo / vpravo   Období"),
    ("Up / Down   Meeting", "Nahoru / dolů   Schůzka"),
    ("Esc   Close", "Esc   Zavřít"),
    ("Tab / Shift+Tab   Focus", "Tab / Shift+Tab   Přechod"),
    ("Reload local cache", "Načíst uložený kalendář"),
    ("MEETING", "SCHŮZKA"),
    ("Close", "Zavřít"),
    ("Guests", "Hosté"),
    ("People", "Účastníci"),
    ("Researching...", "Zjišťuji souvislosti…"),
    ("Research now", "Zjistit souvislosti"),
    ("Insights", "Přehled"),
    ("Expand", "Rozbalit"),
    ("Meeting Insights", "Přehled ke schůzce"),
    ("Prepare", "Co připravit"),
    ("Talking points", "Témata k diskusi"),
    ("Open questions", "Otevřené otázky"),
    ("Risks", "Na co si dát pozor"),
    ("Evidence gaps", "Co se nepodařilo ověřit"),
    ("SOURCES", "ZDROJE"),
    ("Sources", "Zdroje"),
    ("Retrieved context", "Dohledané souvislosti"),
    ("Source", "Zdroj"),
    ("Untitled source", "Zdroj bez názvu"),
    (
        "Referenced in captured results",
        "Odkaz nalezen v dohledaných výsledcích",
    ),
    (
        "A matching URL is not verification of the source's claims.",
        "Nalezený odkaz sám o sobě nepotvrzuje tvrzení zdroje.",
    ),
    (
        "Not found in captured results",
        "Odkaz není v uložených výsledcích",
    ),
    (
        "Capture is bounded. This does not mean the source does not exist.",
        "Ukládáme omezené množství výsledků. Zdroj přesto může existovat.",
    ),
    ("Unverified citation", "Neověřený odkaz"),
    ("Retrieval failed", "Načtení se nezdařilo"),
    (
        "Excerpt only, not the complete source.",
        "Jen výňatek, nikoli celý zdroj.",
    ),
    ("Open cited source", "Otevřít původní zdroj"),
    (
        "Automatic Insights are disabled.",
        "Automatické přehledy jsou vypnuté.",
    ),
    (
        "Automatic Insights are paused.",
        "Automatické přehledy jsou pozastavené.",
    ),
    (
        "AI is not connected yet; Research now queues this meeting.",
        "AI zatím není připojena. Schůzku můžete zařadit do fronty.",
    ),
    ("Retry Insights", "Zkusit znovu"),
    (
        "Previous saved Insights; research is not complete yet.",
        "Dříve uložený přehled. Nový výzkum ještě není hotový.",
    ),
    (
        "No saved Insights yet. Research now prioritizes this meeting.",
        "Přehled zatím není hotový. Zjištění souvislostí dá této schůzce přednost.",
    ),
    (
        "Czech version is being prepared in the background.",
        "Česká verze se připravuje na pozadí.",
    ),
    ("Overview", "To podstatné"),
    ("Next steps", "Další kroky"),
    ("Questions", "Otázky"),
    ("Evidence", "Podklady"),
    ("ASSISTANT", "ASISTENT"),
    ("Your workspace", "Váš pracovní prostor"),
    (
        "Switch to global question",
        "Zeptat se bez vazby na schůzku",
    ),
    ("Ask a question...", "Na co se chcete zeptat?"),
    ("Suggest drafts", "Navrhovat koncepty"),
    (
        "Enter to ask; Shift+Enter for a new line",
        "Enter odešle dotaz, Shift+Enter vloží nový řádek",
    ),
    ("Cancel", "Zrušit"),
    (
        "Shorten the question to 8,000 bytes.",
        "Zkraťte dotaz na nejvýše 8 000 bajtů.",
    ),
    ("No sources retrieved.", "Nebyly dohledány žádné zdroje."),
    (
        "Check important claims against the original sources.",
        "Důležitá tvrzení ověřte v původních zdrojích.",
    ),
    ("Retrieved", "Načteno"),
    (
        "No structured source URL captured.",
        "Výsledek neobsahuje odkaz na původní zdroj.",
    ),
    (
        "Excerpt truncated; this is not the complete response.",
        "Výňatek je zkrácený, nejde o celou odpověď.",
    ),
    (
        "Working asynchronously - the calendar remains available",
        "Pracuji na pozadí – kalendář můžete dál používat",
    ),
    ("New local draft", "Nový místní koncept"),
    ("Title", "Název"),
    ("Local draft", "Místní koncept"),
    (
        "Proposed target - verify manually",
        "Navržený cíl – ověřte ručně",
    ),
    (
        "Draft text / proposed change",
        "Text konceptu / navržená změna",
    ),
    ("Mark reviewed locally", "Označit jako zkontrolované"),
    ("Discard local draft", "Zahodit místní koncept"),
    ("Keep local draft", "Uložit místní koncept"),
    (
        "Kept only in this context's memory; cleared on Exit.",
        "Jen v paměti tohoto kontextu. Ukončení aplikace koncept smaže.",
    ),
    (
        "Memory only. Targets are unverified. Nothing is sent, scheduled or created remotely.",
        "Jen v paměti. Cíle nejsou ověřené. Nic se neodesílá ani nevytváří v jiných službách.",
    ),
    (
        "Meeting changed since this draft was kept; review the target and text.",
        "Schůzka se změnila. Zkontrolujte cíl i text konceptu.",
    ),
    (
        "Model suggestion, not verified or approved. Keeping it has no remote effect.",
        "Návrh AI není ověřený ani schválený. Uložení konceptu nic nemění v jiných službách.",
    ),
    ("AI connection", "Připojení AI"),
    ("WorkIQ connection", "Připojení WorkIQ"),
    ("WebIQ connection", "Připojení WebIQ"),
    ("GitHub connection", "Připojení GitHubu"),
    (
        "Connector has no supported verification read",
        "Konektor nenabízí podporovaný nástroj pro ověření přístupu.",
    ),
    (
        "GitHub MCP did not expose the account verification tool (get_me). Check the context toolset; this is not a rejected sign-in.",
        "GitHub MCP nenabízí nástroj pro ověření účtu (get_me). Zkontrolujte sadu nástrojů context; nejde o odmítnuté přihlášení.",
    ),
    ("Cancel AI sign-in", "Zrušit přihlašování"),
    ("Sign in", "Přihlásit se"),
    ("Not connected", "Nepřipojeno"),
    ("Connected", "Připojeno"),
    ("Browser sign-in", "Přihlášení v prohlížeči"),
    ("Personal token", "Osobní token"),
    ("API key", "Klíč API"),
    ("Personal access token", "Osobní přístupový token"),
    ("GitHub App Client ID", "Client ID aplikace GitHub"),
    ("Create token", "Vytvořit token"),
    ("GitHub App setup", "Registrace aplikace GitHub"),
    ("Connect and save key", "Připojit a uložit klíč"),
    ("Connect and save token", "Připojit a uložit token"),
    ("Disconnect and forget", "Odpojit a smazat přihlášení"),
    ("Remove saved sign-in", "Smazat uložené přihlášení"),
    ("Keep sign-in", "Ponechat přihlášení"),
    (
        "Remove this saved sign-in from Tomlook? Your provider account is not deleted.",
        "Smazat uložené přihlášení z Tomlooku? Účet u poskytovatele zůstane zachovaný.",
    ),
    ("Work email", "Pracovní e-mail"),
    (
        "Disconnect and forget sign-in",
        "Odpojit a smazat přihlášení",
    ),
    ("Forget saved sign-in", "Smazat uložené přihlášení"),
    (
        "The key is encrypted for your Windows account. Public web queries only.",
        "Klíč je zašifrovaný pro váš účet Windows. Slouží jen k veřejnému webovému hledání.",
    ),
    (
        "No app registration. Use a fine-grained token for selected repositories with Contents, Issues and Pull requests set to Read-only.",
        "Registrace aplikace není potřeba. Vytvořte token pro vybrané repozitáře a nastavte Contents, Issues a Pull requests na Read-only.",
    ),
    (
        "Use a GitHub App with Device flow enabled and read-only access to selected repositories. No client secret is needed.",
        "Použijte aplikaci GitHub s povoleným Device flow a čtením vybraných repozitářů. Client secret není potřeba.",
    ),
    ("Insights activity", "Průběh zjišťování"),
    ("Automatic Insights", "Automatické přehledy"),
    ("Pause Insights", "Pozastavit přehledy"),
    ("All", "Vše"),
    ("Working", "Probíhá"),
    ("Waiting", "Čeká"),
    ("Problems", "Potíže"),
    ("Ready", "Hotovo"),
    ("Later", "Později"),
    ("Researching", "Zjišťuji souvislosti"),
    ("Failed", "Nezdařilo se"),
    ("Needs retry", "Zkusit znovu"),
    ("Cancelled", "Zrušeno"),
    (
        "Interrupted provider operation; inspect saved evidence and explicitly retry before resuming preparation",
        "Zjišťování bylo přerušeno. Prohlédněte uložené podklady a teprve potom je výslovně spusťte znovu.",
    ),
    (
        "Interrupted provider operation; inspect saved evidence before explicit retry",
        "Zjišťování bylo přerušeno. Před novým pokusem prohlédněte uložené podklady.",
    ),
    (
        "credential rejected; check the key or sign-in",
        "přihlášení bylo odmítnuto; zkontrolujte klíč nebo se znovu přihlaste",
    ),
    (
        "access denied; check provider permissions",
        "přístup byl zamítnut; zkontrolujte oprávnění u poskytovatele",
    ),
    (
        "provider rate limit; retry later",
        "poskytovatel omezil počet požadavků; zkuste to později",
    ),
    (
        "provider unavailable; retry later",
        "poskytovatel je nedostupný; zkuste to později",
    ),
    ("provider request failed", "požadavek se nezdařil"),
    ("transport failed", "spojení se nezdařilo"),
    (
        "HTTP request timed out",
        "vypršel časový limit požadavku HTTP",
    ),
    ("cannot reach the provider", "poskytovatel je nedostupný"),
    (
        "provider returned an invalid MCP response",
        "poskytovatel vrátil neplatnou odpověď MCP",
    ),
    (
        "provider returned an unsupported response type",
        "poskytovatel vrátil nepodporovaný typ odpovědi",
    ),
    (
        "MCP session expired; reconnect",
        "relace MCP vypršela; připojte se znovu",
    ),
    (
        "provider returned invalid MCP data",
        "poskytovatel vrátil neplatná data MCP",
    ),
    ("transport/protocol failed", "chyba spojení nebo protokolu"),
    (
        "no compatible MCP protocol",
        "žádná podporovaná verze protokolu MCP",
    ),
    (
        "invalid MCP initialization response",
        "neplatná odpověď při navazování spojení MCP",
    ),
    (
        "invalid tool arguments (MCP code -32602); not a rejected key",
        "neplatné parametry nástroje (kód MCP -32602); nejde o odmítnutí klíče",
    ),
    (
        "MCP transport closed before the response",
        "spojení MCP bylo ukončeno před přijetím odpovědi",
    ),
    (
        "MCP request timed out",
        "vypršel časový limit požadavku MCP",
    ),
    (
        "provider returned unexpected MCP data",
        "poskytovatel vrátil neočekávaná data MCP",
    ),
    ("MCP request failed", "požadavek MCP se nezdařil"),
    (" (partial)", " (neúplné)"),
    (" - source errors", " – chyby zdrojů"),
    (" (partial) - source errors", " (neúplné) – chyby zdrojů"),
    ("No matching jobs.", "Žádné odpovídající schůzky."),
    (
        "Repository collections (one per line)",
        "Složky s repozitáři (jedna na řádek)",
    ),
    ("Local document folders", "Místní složky s dokumenty"),
    (
        "Customer folder in OneDrive (WorkIQ)",
        "Složka zákazníků na OneDrivu (WorkIQ)",
    ),
    ("Save sources", "Uložit zdroje"),
    (
        "Resume other queued jobs",
        "Pokračovat s ostatními schůzkami",
    ),
    ("Exit Tomlook", "Ukončit Tomlook"),
    ("Dismiss", "Zavřít"),
    ("GitHub Copilot: signing in...", "GitHub Copilot: přihlašuji…"),
    ("GitHub Copilot: connected", "GitHub Copilot: připojeno"),
    ("GitHub Copilot: disconnected", "GitHub Copilot: nepřipojeno"),
    ("WorkIQ: working...", "WorkIQ: pracuje…"),
    ("WorkIQ: connected", "WorkIQ: připojeno"),
    ("WorkIQ: disconnected", "WorkIQ: nepřipojeno"),
    ("connecting...", "připojuji…"),
    ("disconnecting...", "odpojuji…"),
    ("connected", "připojeno"),
    ("disconnected", "nepřipojeno"),
    ("Connecting...", "Připojuji…"),
    ("Open Tomlook", "Otevřít Tomlook"),
    ("Exit Tomlook (stop AI)", "Ukončit Tomlook (zastavit AI)"),
    ("Pause / resume Insights", "Pozastavit / obnovit přehledy"),
    (
        "Tomlook - calendar and Insights",
        "Tomlook – kalendář a přehledy",
    ),
    ("Select a meeting first.", "Nejdřív vyberte schůzku."),
    ("Connect AI before asking.", "Nejdřív připojte AI."),
    ("Show day", "Zobrazit den"),
    ("Show work week", "Zobrazit pracovní týden"),
    ("Show week", "Zobrazit týden"),
    ("Show month", "Zobrazit měsíc"),
    ("Go to today", "Přejít na dnešek"),
    ("Ask a question", "Položit dotaz"),
    (
        "Research selected meeting now",
        "Zjistit souvislosti k vybrané schůzce",
    ),
    ("Toggle dark / light", "Přepnout světlý / tmavý vzhled"),
    ("Refresh calendar", "Obnovit kalendář"),
    (
        "Select at most 42 consecutive days.",
        "Vyberte nejvýše 42 po sobě jdoucích dnů.",
    ),
    ("No synchronized days yet.", "Zatím nebyl načten žádný den."),
    (
        "Show meetings, queue and local context",
        "Schůzky, fronta a místní zdroje",
    ),
    (
        "Local calendar cache; no connected account assigned.",
        "Místní kalendář. Zatím není přiřazen připojený účet.",
    ),
    (
        "Participant list is limited to 500. Open the original meeting for the full list.",
        "Zobrazujeme nejvýše 500 účastníků. Úplný seznam najdete v původní schůzce.",
    ),
    ("Open original meeting", "Otevřít původní schůzku"),
    (
        "Model-supplied citations; not independently verified",
        "Odkazy navržené AI nejsou nezávisle ověřené",
    ),
    (
        "Tool results captured; no cited links. Claims remain unverified.",
        "Výsledky byly uloženy, ale bez odkazů. Tvrzení zůstávají neověřená.",
    ),
    ("Updates", "Aktualizace"),
    ("Current version", "Aktuální verze"),
    (
        "Check for updates automatically once a day",
        "Automaticky kontrolovat aktualizace jednou denně",
    ),
    ("Check now", "Zkontrolovat nyní"),
    ("Not checked yet.", "Zatím nezkontrolováno."),
    ("Checking GitHub Releases...", "Kontroluji GitHub Releases…"),
    ("Restart now", "Restartovat nyní"),
    ("Release notes", "Poznámky k vydání"),
    (
        "Otherwise the update is installed when you exit Tomlook.",
        "Jinak se aktualizace nainstaluje při ukončení Tomlooku.",
    ),
    ("Open release page", "Otevřít stránku vydání"),
    ("Tomlook is up to date.", "Tomlook je aktuální."),
    (
        "Tomlook makes at most one unauthenticated request per day to the GitHub Releases API. No account, calendar or usage data is sent.",
        "Tomlook se nejvýše jednou denně anonymně dotáže rozhraní GitHub Releases API. Neodesílá žádné údaje o účtu, kalendáři ani používání.",
    ),
    (
        "An update is installed only with a valid Tomlook release signature and matching SHA-256 hashes.",
        "Aktualizace se nainstaluje jen s platným podpisem vydání Tomlooku a se shodnými otisky SHA-256.",
    ),
    (
        "A verified update is ready. Restart from Settings, or it is installed when you exit Tomlook.",
        "Ověřená aktualizace je připravená. Restartujte Tomlook v Nastavení, jinak se nainstaluje při jeho ukončení.",
    ),
    (
        "Updates are not available in this session.",
        "Aktualizace nejsou v této relaci k dispozici.",
    ),
    (
        "Updates are disabled in the offline demo.",
        "V offline ukázce jsou aktualizace vypnuté.",
    ),
    (
        "Updates are disabled in measurement and diagnostic modes.",
        "V měřicích a diagnostických režimech jsou aktualizace vypnuté.",
    ),
    (
        "Updates cannot be installed on exit in this session.",
        "V této relaci nelze aktualizaci nainstalovat při ukončení.",
    ),
    (
        "The update checker is busy; try again in a moment.",
        "Kontrola aktualizací je zaneprázdněná; zkuste to za chvíli.",
    ),
    (
        "The update signature is invalid; the update was rejected.",
        "Podpis aktualizace je neplatný; aktualizace byla odmítnuta.",
    ),
    (
        "The downloaded archive does not match its signed SHA-256; it was discarded.",
        "Stažený archiv neodpovídá podepsanému otisku SHA-256; byl zahozen.",
    ),
    (
        "The extracted Tomlook.exe does not match its signed SHA-256; it was discarded.",
        "Rozbalený Tomlook.exe neodpovídá podepsanému otisku SHA-256; byl zahozen.",
    ),
    (
        "The update archive contains an unsafe path; it was rejected.",
        "Archiv aktualizace obsahuje nebezpečnou cestu; byl odmítnut.",
    ),
    (
        "The update archive contains an unexpected entry; it was rejected.",
        "Archiv aktualizace obsahuje neočekávanou položku; byl odmítnut.",
    ),
    (
        "GitHub Releases could not be reached; check the network and try again.",
        "GitHub Releases není dostupný; zkontrolujte připojení a zkuste to znovu.",
    ),
    (
        "GitHub API rate limit reached; Tomlook will try again later.",
        "GitHub API dočasně omezil počet požadavků; Tomlook to zkusí později.",
    ),
    (
        "No published Tomlook release was found.",
        "Nenašlo se žádné zveřejněné vydání Tomlooku.",
    ),
    (
        "The latest release has no signed update information.",
        "Nejnovější vydání nemá podepsané informace o aktualizaci.",
    ),
    (
        "A GitHub response exceeded its size limit; the update was rejected.",
        "Odpověď GitHubu překročila povolenou velikost; aktualizace byla odmítnuta.",
    ),
    (
        "Signed update information does not match the GitHub release; the update was rejected.",
        "Podepsané informace o aktualizaci neodpovídají vydání na GitHubu; aktualizace byla odmítnuta.",
    ),
    (
        "GitHub returned an unreadable release description.",
        "GitHub vrátil nečitelný popis vydání.",
    ),
    (
        "Signed update information is not readable.",
        "Podepsané informace o aktualizaci nejsou čitelné.",
    ),
    (
        "Signed update information is invalid; the update was rejected.",
        "Podepsané informace o aktualizaci jsou neplatné; aktualizace byla odmítnuta.",
    ),
    (
        "The update archive could not be read; it was discarded.",
        "Archiv aktualizace nelze přečíst; byl zahozen.",
    ),
    (
        "The update archive does not contain Tomlook.exe.",
        "Archiv aktualizace neobsahuje Tomlook.exe.",
    ),
    (
        "The update could not be saved next to Tomlook.exe.",
        "Aktualizaci nelze uložit vedle souboru Tomlook.exe.",
    ),
    (
        "The update download stopped because Tomlook is closing.",
        "Stahování aktualizace se zastavilo, protože se Tomlook ukončuje.",
    ),
    (
        "Tomlook can replace itself only as Tomlook.exe; download the update from the release page.",
        "Tomlook se umí nahradit jen jako Tomlook.exe; stáhněte aktualizaci ze stránky vydání.",
    ),
    (
        "Tomlook cannot write to its own folder; download the update from the release page.",
        "Tomlook nemůže zapisovat do své složky; stáhněte aktualizaci ze stránky vydání.",
    ),
    (
        "The staged update changed after verification and was not installed.",
        "Připravená aktualizace se po ověření změnila a nebyla nainstalována.",
    ),
    (
        "A previous Tomlook update is still in use; exit all Tomlook windows and try again.",
        "Předchozí aktualizace Tomlooku se stále používá; ukončete všechna okna Tomlooku a zkuste to znovu.",
    ),
    (
        "Development builds are not updated automatically; rebuild them instead.",
        "Vývojová sestavení se automaticky neaktualizují; sestavte je znovu.",
    ),
    (
        "The update checker could not start.",
        "Kontrolu aktualizací nelze spustit.",
    ),
    ("Notifications", "Oznámení"),
    ("Meeting reminders", "Připomínky schůzek"),
    ("Remind me before meetings", "Připomínat schůzky předem"),
    ("Minutes before the start", "Kolik minut před začátkem"),
    (
        "Also remind me of tentatively accepted meetings",
        "Připomínat i nezávazně přijaté schůzky",
    ),
    (
        "Reminders cover timed meetings you organize or have accepted. Unanswered, declined, cancelled, free and followed meetings are skipped.",
        "Připomínky se týkají schůzek s časem, které pořádáte nebo jste přijali. Nezodpovězené, odmítnuté, zrušené, sledované a jako volno označené schůzky se přeskakují.",
    ),
    (
        "A reminder shows up to three points from saved Insights and never starts AI research.",
        "Připomínka ukáže až tři body z uloženého přehledu a nikdy nespouští zjišťování pomocí AI.",
    ),
    ("Send test notification", "Odeslat zkušební oznámení"),
    (
        "Shows a sample reminder with fictional content.",
        "Zobrazí ukázkovou připomínku se smyšleným obsahem.",
    ),
    (
        "Tomlook is listed in Windows Settings > System > Notifications, where you can turn its notifications off or change how they appear.",
        "Tomlook najdete v Nastavení Windows > Systém > Oznámení, kde můžete jeho oznámení vypnout nebo změnit, jak se zobrazují.",
    ),
    ("Meeting reminders are turned off.", "Připomínky schůzek jsou vypnuté."),
    ("No upcoming reminders.", "Žádné nadcházející připomínky."),
    (
        "Windows notifications are on for Tomlook.",
        "Oznámení Windows jsou pro Tomlook zapnutá.",
    ),
    (
        "Windows notifications are turned off for Tomlook. Turn them on in Windows Settings > System > Notifications.",
        "Oznámení Windows jsou pro Tomlook vypnutá. Zapněte je v Nastavení Windows > Systém > Oznámení.",
    ),
    (
        "Windows notifications are turned off for this user.",
        "Oznámení Windows jsou pro tohoto uživatele vypnutá.",
    ),
    (
        "Windows notifications are blocked by group policy.",
        "Oznámení Windows blokují zásady skupiny.",
    ),
    (
        "Windows does not allow notifications for Tomlook.",
        "Windows Tomlooku oznámení nepovoluje.",
    ),
    (
        "Windows reported an unknown notification setting.",
        "Windows hlásí neznámé nastavení oznámení.",
    ),
    (
        "Meeting reminders are disabled in the offline demo; the test notification uses fictional content.",
        "Připomínky schůzek jsou v offline ukázce vypnuté; zkušební oznámení používá smyšlený obsah.",
    ),
    (
        "Notifications are disabled in measurement and diagnostic modes.",
        "Oznámení jsou v režimech měření a diagnostiky vypnutá.",
    ),
    (
        "Notifications are not available in this session.",
        "Oznámení nejsou v této relaci k dispozici.",
    ),
    (
        "Meeting reminder shutdown could not be verified",
        "Ukončení připomínek schůzek nelze ověřit",
    ),
    (
        "Meeting reminders stopped unexpectedly",
        "Připomínky schůzek se neočekávaně zastavily",
    ),
    (
        "Read meeting reminder history failed",
        "Načtení historie připomínek se nezdařilo",
    ),
    (
        "Saved meeting reminder history is invalid",
        "Uložená historie připomínek je neplatná",
    ),
    (
        "Meeting reminder history exceeds its safety limit",
        "Historie připomínek překročila bezpečnostní limit",
    ),
    (
        "Save meeting reminder history failed",
        "Uložení historie připomínek se nezdařilo",
    ),
    (
        "Encode meeting reminder history failed",
        "Zakódování historie připomínek se nezdařilo",
    ),
    ("Join", "Připojit"),
    ("Open", "Otevřít"),
    ("1 minute", "1 minuta"),
    ("2 minutes", "2 minuty"),
    ("5 minutes", "5 minut"),
    ("10 minutes", "10 minut"),
    ("15 minutes", "15 minut"),
    ("Test notification", "Zkušební oznámení"),
    ("Sample meeting (fictional)", "Ukázková schůzka (smyšlená)"),
    ("Room 2 (sample)", "Místnost 2 (ukázka)"),
    (
        "Prepare: Review the sample agenda",
        "Příprava: Projděte ukázkový program",
    ),
    (
        "Talking point: Confirm the next step",
        "Téma: Potvrďte další krok",
    ),
    (
        "Question: Who owns the follow-up?",
        "Otázka: Kdo převezme navazující úkol?",
    ),
    ("Helpful", "Užitečné"),
    ("Not helpful", "Neužitečné"),
    ("Click again to clear the rating.", "Dalším kliknutím hodnocení zrušíte."),
    ("What was not helpful?", "Co nebylo užitečné?"),
    ("Inaccurate", "Nepřesné"),
    ("Not relevant", "Nerelevantní"),
    ("Missing something important", "Chybí něco důležitého"),
    ("Too long", "Příliš dlouhé"),
    ("Too vague", "Příliš obecné"),
    ("Wrong language", "Špatný jazyk"),
    ("Other", "Jiné"),
    ("Whole document", "Celý dokument"),
    (
        "Comment (optional, up to 500 characters)",
        "Komentář (nepovinný, nejvýše 500 znaků)",
    ),
    ("Save", "Uložit"),
    (
        "Feedback stays on this computer and is never sent automatically.",
        "Zpětná vazba zůstává v tomto počítači a nikdy se automaticky neodesílá.",
    ),
    ("Feedback on AI output", "Zpětná vazba na výstupy AI"),
    (
        "Feedback is collected for the Before the meeting and After the meeting prompts.",
        "Zpětná vazba se sbírá pro prompty Před schůzkou a Po schůzce.",
    ),
    (
        "Ratings from the last 90 days. They stay on this computer and are never sent automatically.",
        "Hodnocení za posledních 90 dní. Zůstávají v tomto počítači a nikdy se automaticky neodesílají.",
    ),
    (
        "Feedback is available once the calendar account is known.",
        "Zpětná vazba bude dostupná, jakmile bude známý účet kalendáře.",
    ),
    ("Loading feedback...", "Načítám zpětnou vazbu…"),
    (
        "No feedback in the last 90 days. Rate Insights or meeting notes with the thumb icons.",
        "Za posledních 90 dní tu není žádná zpětná vazba. Hodnoťte přehledy a zápisy ze schůzek ikonami palce.",
    ),
    ("Top reasons:", "Nejčastější důvody:"),
    ("Most criticized sections:", "Nejčastěji kritizované části:"),
    ("Recent comments", "Nedávné komentáře"),
    (
        "A suggestion needs at least one negative rating.",
        "Návrh vyžaduje alespoň jedno negativní hodnocení.",
    ),
    (
        "Offline demo never sends AI requests.",
        "Offline demo nikdy neodesílá požadavky na AI.",
    ),
    (
        "Connect GitHub Copilot to suggest prompt improvements.",
        "Pro návrh vylepšení promptu připojte GitHub Copilot.",
    ),
    (
        "Connect GitHub Copilot first. No AI request was made.",
        "Nejdřív připojte GitHub Copilot. Žádný požadavek na AI nebyl odeslán.",
    ),
    ("Suggest prompt improvement", "Navrhnout vylepšení promptu"),
    (
        "Sends the editor's prompt and summarized feedback to GitHub Copilot once (Thorough profile, no tools). Counts toward the daily AI limit.",
        "Jednou odešle prompt z editoru a souhrn zpětné vazby do GitHub Copilotu (profil Důkladně, bez nástrojů). Započítá se do denního limitu AI.",
    ),
    ("Preparing a prompt suggestion...", "Připravuji návrh promptu…"),
    ("Suggested prompt (draft)", "Navržený prompt (koncept)"),
    (
        "Use draft copies it into the editor above; nothing is saved until you click Save prompts.",
        "Tlačítko Použít koncept jej zkopíruje do editoru výše; nic se neuloží, dokud nekliknete na Uložit prompty.",
    ),
    ("Why these changes", "Proč tyto změny"),
    ("Changes", "Změny"),
    (
        "The editor changed after this suggestion was requested; Use draft replaces the editor text.",
        "Text v editoru se od vyžádání návrhu změnil; Použít koncept jej nahradí.",
    ),
    ("Suggested prompt", "Navržený prompt"),
    ("Use draft", "Použít koncept"),
    ("Discard", "Zahodit"),
    (
        "Prompt suggestions use feedback on Insights and meeting notes only.",
        "Návrhy promptů vycházejí jen ze zpětné vazby na přehledy a zápisy ze schůzek.",
    ),
    (
        "Fix the current prompt before requesting a suggestion.",
        "Před vyžádáním návrhu opravte současný prompt.",
    ),
    (
        "Feedback exceeds the prompt suggestion input limit; nothing was submitted.",
        "Zpětná vazba překračuje vstupní limit návrhu promptu; nic nebylo odesláno.",
    ),
    (
        "AI returned an oversized prompt suggestion; nothing was changed.",
        "AI vrátila příliš dlouhý návrh promptu; nic se nezměnilo.",
    ),
    (
        "AI returned an invalid prompt suggestion; nothing was changed.",
        "AI vrátila neplatný návrh promptu; nic se nezměnilo.",
    ),
    (
        "AI returned an empty prompt suggestion; nothing was changed.",
        "AI vrátila prázdný návrh promptu; nic se nezměnilo.",
    ),
    (
        "The suggested prompt exceeds 4,000 bytes or contains unsupported characters; nothing was changed.",
        "Navržený prompt překračuje 4 000 bajtů nebo obsahuje nepodporované znaky; nic se nezměnilo.",
    ),
    (
        "The suggestion did not change the prompt; nothing was changed.",
        "Návrh prompt nijak neupravil; nic se nezměnilo.",
    ),
    (
        "AI returned a suggestion without a rationale; nothing was changed.",
        "AI vrátila návrh bez zdůvodnění; nic se nezměnilo.",
    ),
    (
        "Feedback is invalid or exceeds its bounds; nothing was saved",
        "Zpětná vazba je neplatná nebo překračuje limity; nic nebylo uloženo.",
    ),
    (
        "Feedback no longer matches the displayed document; nothing was saved",
        "Zpětná vazba už neodpovídá zobrazenému dokumentu; nic nebylo uloženo.",
    ),
    (
        "Feedback belongs to a different calendar account; nothing was saved",
        "Zpětná vazba patří k jinému účtu kalendáře; nic nebylo uloženo.",
    ),
    (
        "Saved feedback reached its safety limit; nothing was saved",
        "Uložená zpětná vazba dosáhla bezpečnostního limitu; nic nebylo uloženo.",
    ),
    ("Saved feedback is invalid", "Uložená zpětná vazba je neplatná."),
    ("Save feedback failed", "Zpětnou vazbu se nepodařilo uložit."),
    (
        "Daily AI request limit reached; no prompt suggestion was requested.",
        "Denní limit požadavků na AI je vyčerpán; návrh promptu nebyl vyžádán.",
    ),
    (
        "Copilot reported a session error; no prompt suggestion was produced.",
        "Copilot nahlásil chybu relace; návrh promptu nevznikl.",
    ),
    (
        "Prompt suggestion cancelled; no draft was produced.",
        "Návrh promptu byl zrušen; koncept nevznikl.",
    ),
    (
        "The prompt or feedback contains an app credential; nothing was submitted",
        "Prompt nebo zpětná vazba obsahuje přihlašovací údaj aplikace; nic nebylo odesláno.",
    ),
    ("Tasks", "Úkoly"),
    ("Task", "Úkol"),
    ("Open tasks", "Otevřít úkoly"),
    ("Add task", "Přidat úkol"),
    ("Add", "Přidat"),
    ("Add a task and press Enter", "Napište úkol a stiskněte Enter"),
    (
        "A local list on this computer. Nothing is sent to To Do, Planner or Dataverse.",
        "Místní seznam v tomto počítači. Nic se neodesílá do To Do, Planneru ani Dataverse.",
    ),
    (
        "Tasks are available once the calendar account is known.",
        "Úkoly budou k dispozici, jakmile bude znám účet kalendáře.",
    ),
    ("Task deleted", "Úkol smazán"),
    ("Undo", "Vrátit zpět"),
    (
        "No open tasks. Tasks from meeting notes appear here automatically.",
        "Žádné otevřené úkoly. Úkoly ze zápisů ze schůzek se tu objeví automaticky.",
    ),
    ("Overdue", "Po termínu"),
    ("Due today", "Termín dnes"),
    ("Upcoming", "Nadcházející"),
    ("No due date", "Bez termínu"),
    ("Done", "Hotovo"),
    ("Done in the last 30 days", "Hotovo za posledních 30 dní"),
    ("Added manually", "Přidáno ručně"),
    ("From meeting notes", "Ze zápisu ze schůzky"),
    ("From Ask", "Z odpovědi AI"),
    ("Mark as done", "Označit jako hotové"),
    ("Mark as not done", "Označit jako nehotové"),
    ("Tomorrow", "Zítra"),
    ("Yesterday", "Včera"),
    ("Next week", "Příští týden"),
    ("Set due date", "Nastavit termín"),
    ("Due", "Termín"),
    ("Due date", "Termín splnění"),
    ("Edit", "Upravit"),
    ("Delete the task; you can undo it", "Smazat úkol (lze vrátit zpět)"),
    ("Note (optional)", "Poznámka (nepovinná)"),
    ("In my task list", "V mém seznamu úkolů"),
    ("Open meeting details", "Otevřít podrobnosti schůzky"),
    (
        "Task title must have 1-300 characters on one line",
        "Název úkolu musí mít 1–300 znaků na jednom řádku",
    ),
    (
        "Task note can have at most 2,000 characters",
        "Poznámka k úkolu může mít nejvýše 2 000 znaků",
    ),
    ("Task no longer exists", "Úkol už neexistuje"),
    (
        "The task list reached its limit of 5,000 tasks; delete completed tasks first",
        "Seznam úkolů dosáhl limitu 5 000 úkolů; nejprve smažte hotové úkoly",
    ),
    (
        "This belongs to a different calendar account; nothing was changed",
        "Patří k jinému účtu kalendáře; nic se nezměnilo",
    ),
    ("Task was already restored", "Úkol už byl obnoven"),
    (
        "Only task drafts can be added to tasks",
        "Do úkolů lze přidat jen koncepty úkolů",
    ),
    ("Follow-up e-mail", "Follow-up e-mail"),
    ("Prepare follow-up e-mail", "Připravit follow-up e-mail"),
    ("Preparing the follow-up e-mail...", "Připravuji follow-up e-mail…"),
    ("Open follow-up e-mail", "Otevřít follow-up e-mail"),
    (
        "Tomlook recommends a follow-up for this meeting.",
        "Tomlook u této schůzky doporučuje follow-up.",
    ),
    (
        "Local text only: Tomlook never sends e-mail or creates Outlook drafts.",
        "Jen místní text: Tomlook nikdy neodesílá e-maily ani nevytváří koncepty v Outlooku.",
    ),
    (
        "Drafts the e-mail from these notes with one Quick AI request. Nothing is sent.",
        "Připraví e-mail z tohoto zápisu jedním rychlým dotazem na AI. Nic se neodesílá.",
    ),
    (
        "No participant has an e-mail address; add recipients in your e-mail app.",
        "Žádný účastník nemá e-mailovou adresu; příjemce doplňte v e-mailové aplikaci.",
    ),
    ("Subject", "Předmět"),
    ("Message", "Zpráva"),
    ("External", "Externí"),
    ("Copy", "Kopírovat"),
    ("Open in Outlook", "Otevřít v Outlooku"),
    ("Regenerate", "Vygenerovat znovu"),
    (
        "Copies the subject and message to the clipboard",
        "Zkopíruje předmět a zprávu do schránky",
    ),
    (
        "Opens a new message in your default e-mail app. Nothing is sent automatically.",
        "Otevře novou zprávu ve vaší výchozí e-mailové aplikaci. Nic se neodešle automaticky.",
    ),
    (
        "Replaces this text with a new AI draft",
        "Nahradí tento text novým návrhem od AI",
    ),
    ("Copied to the clipboard.", "Zkopírováno do schránky."),
    (
        "Opened in your e-mail app. Nothing was sent.",
        "Otevřeno v e-mailové aplikaci. Nic nebylo odesláno.",
    ),
    (
        "The message is too long for an e-mail link. The body was copied to the clipboard; paste it into the new message.",
        "Zpráva je pro e-mailový odkaz příliš dlouhá. Text byl zkopírován do schránky; vložte ho do nové zprávy.",
    ),
    (
        "Follow-up e-mail cancelled; nothing was saved.",
        "Příprava follow-up e-mailu byla zrušena; nic nebylo uloženo.",
    ),
    (
        "Meeting notes are ready; prepare the follow-up e-mail from them in the meeting details.",
        "Zápis ze schůzky je hotový; follow-up e-mail z něj připravíte v podrobnostech schůzky.",
    ),
    (
        "Drafts stay local until you use them; nothing is sent.",
        "Koncepty zůstávají jen v tomto počítači, dokud je nepoužijete; nic se neodesílá.",
    ),
    ("Add to tasks", "Přidat do úkolů"),
    ("Added to tasks", "Přidáno do úkolů"),
    (
        "A follow-up e-mail needs saved meeting notes",
        "Follow-up e-mail vyžaduje uložený zápis ze schůzky",
    ),
    (
        "Meeting notes exceed the follow-up e-mail input limit; nothing was submitted",
        "Zápis ze schůzky překračuje vstupní limit follow-up e-mailu; nic nebylo odesláno",
    ),
    (
        "The e-mail subject must have 1-200 characters on one line",
        "Předmět e-mailu musí mít 1–200 znaků na jednom řádku",
    ),
    (
        "The e-mail body must have 1-4,000 characters",
        "Text e-mailu musí mít 1–4 000 znaků",
    ),
    (
        "Saved follow-up e-mail is invalid",
        "Uložený follow-up e-mail je neplatný",
    ),
    (
        "Only plain e-mail addresses can be used in an e-mail link",
        "V e-mailovém odkazu lze použít jen samotné e-mailové adresy",
    ),
    (
        "Too many recipients for an e-mail link; use Copy instead",
        "Příliš mnoho příjemců pro e-mailový odkaz; použijte raději Kopírovat",
    ),
    (
        "Only e-mail links with plain addresses, a subject and a body can be opened",
        "Otevřít lze jen e-mailové odkazy s adresami, předmětem a textem",
    ),
    (
        "AI returned an oversized follow-up e-mail; nothing was saved.",
        "AI vrátila příliš dlouhý follow-up e-mail; nic nebylo uloženo.",
    ),
    (
        "AI returned an invalid follow-up e-mail; nothing was saved.",
        "AI vrátila neplatný follow-up e-mail; nic nebylo uloženo.",
    ),
    (
        "AI returned an empty follow-up e-mail; nothing was saved.",
        "AI vrátila prázdný follow-up e-mail; nic nebylo uloženo.",
    ),
    (
        "Copilot reported a session error; no follow-up e-mail was produced.",
        "Copilot nahlásil chybu relace; follow-up e-mail nevznikl.",
    ),
    (
        "Daily AI request limit reached; no follow-up e-mail was requested.",
        "Denní limit požadavků na AI je vyčerpán; follow-up e-mail nebyl vyžádán.",
    ),
    (
        "Cancelled before the follow-up e-mail was requested",
        "Zrušeno před vyžádáním follow-up e-mailu",
    ),
    (
        "Cancelled while requesting the follow-up e-mail; provider outcome may be unknown",
        "Zrušeno během vyžádání follow-up e-mailu; výsledek u poskytovatele nemusí být známý",
    ),
    (
        "The follow-up e-mail took too long; try again",
        "Příprava follow-up e-mailu trvala příliš dlouho; zkuste to znovu",
    ),
    (
        "Creating the follow-up e-mail session timed out",
        "Vytvoření relace pro follow-up e-mail vypršelo",
    ),
    (
        "The meeting notes contain an app credential; nothing was submitted",
        "Zápis ze schůzky obsahuje přihlašovací údaj aplikace; nic nebylo odesláno",
    ),
    (
        "AI is stopping; nothing was submitted",
        "AI se zastavuje; nic nebylo odesláno",
    ),
    ("My day", "Můj den"),
    ("Ctrl+D   My day", "Ctrl+D   Můj den"),
    ("Current meeting", "Právě probíhá"),
    ("Next meeting", "Další schůzka"),
    ("No more meetings today.", "Dnes vás už žádná schůzka nečeká."),
    ("Today's agenda", "Dnešní program"),
    ("No meetings today.", "Dnes nemáte žádné schůzky."),
    ("Insights ready", "Přehled je hotový"),
    ("Notes ready", "Zápis je hotový"),
    ("Not prepared", "Bez přípravy"),
    ("Conflicts", "Konflikty"),
    ("No conflicts today.", "Dnes žádné konflikty."),
    (
        "Every meeting that needs preparation today has Insights.",
        "Všechny dnešní schůzky, které potřebují přípravu, mají přehled.",
    ),
    ("Research all", "Zjistit u všech"),
    (
        "Research is not available in the offline demo.",
        "V offline ukázce nelze zjišťovat souvislosti.",
    ),
    (
        "Connect AI and WorkIQ to research these meetings.",
        "Pro zjištění souvislostí připojte AI a WorkIQ.",
    ),
    ("All of them are already queued.", "Všechny už čekají ve frontě."),
    (
        "No missing materials for today's meetings.",
        "Dnešním schůzkám nechybí žádné podklady.",
    ),
    ("Open in Messages", "Otevřít ve Zprávách"),
    (
        "No overdue tasks, tasks due today or tasks from recent notes.",
        "Žádné úkoly po termínu, s dnešním termínem ani z nedávných zápisů.",
    ),
    (
        "No mentions of you in recent meeting notes.",
        "V nedávných zápisech ze schůzek o vás nic není.",
    ),
    ("Focus time", "Čas na soustředění"),
    (
        "No free block of 30 minutes or more is left between 08:00 and 18:00.",
        "Mezi 8:00 a 18:00 už nezbývá volný blok alespoň na 30 minut.",
    ),
    (
        "Free in the confirmed calendar between 08:00 and 18:00.",
        "Volno v potvrzeném kalendáři mezi 8:00 a 18:00.",
    ),
    (
        "Today's calendar is not synced yet, so free time is not shown.",
        "Dnešní kalendář ještě není synchronizovaný, proto volno nezobrazuji.",
    ),
    ("Waiting for approval", "Čeká na schválení"),
    ("Nothing is waiting for approval.", "Nic nečeká na schválení."),
    ("Open Messages", "Otevřít Zprávy"),
    (
        "Tomorrow's calendar is not synced yet.",
        "Zítřejší kalendář ještě není synchronizovaný.",
    ),
    (
        "Today's calendar is not synced yet; the overview may be incomplete.",
        "Dnešní kalendář ještě není synchronizovaný; přehled nemusí být úplný.",
    ),
    ("Morning overview", "Ranní přehled"),
    ("Show my day in the morning", "Ráno zobrazit přehled Můj den"),
    ("Time", "Čas"),
    ("Also on weekends", "I o víkendu"),
    (
        "Once a day (on workdays unless weekends are included) with today's meetings, conflicts, meetings without Insights and tasks. If Tomlook is not running at that time, it appears at the next opportunity before 12:00. It stays quiet when today's calendar is not synced or there is nothing to act on.",
        "Jednou denně (v pracovní dny, pokud nezapnete i víkendy) s dnešními schůzkami, konflikty, schůzkami bez přehledu a úkoly. Pokud Tomlook v tu dobu neběží, ukáže se při nejbližší příležitosti do 12:00. Neozve se, když dnešní kalendář není synchronizovaný nebo není co řešit.",
    ),
    ("Morning overview is turned off.", "Ranní přehled je vypnutý."),
    (
        "The morning overview does not run in the offline demo; the test uses the demo's fictional calendar.",
        "V offline ukázce se ranní přehled nespouští; test použije smyšlený kalendář ukázky.",
    ),
    ("Show now", "Zobrazit hned"),
    ("Send test", "Odeslat test"),
    (
        "Shows the My day notification with the current counts.",
        "Zobrazí oznámení Můj den s aktuálními počty.",
    ),
    (
        "Read morning overview history failed",
        "Načtení historie ranního přehledu se nezdařilo",
    ),
    (
        "Saved morning overview history is invalid",
        "Uložená historie ranního přehledu je neplatná",
    ),
    (
        "Save morning overview history failed",
        "Uložení historie ranního přehledu se nezdařilo",
    ),
    ("Weekly review", "Týdenní přehled"),
    (
        "Open the weekly review of this week",
        "Otevřít týdenní přehled tohoto týdne",
    ),
    ("Review the displayed week", "Přehled zobrazeného týdne"),
    (
        "Future weeks cannot be reviewed yet.",
        "Budoucí týdny zatím nelze vyhodnotit.",
    ),
    ("Previous week", "Předchozí týden"),
    ("This week", "Tento týden"),
    ("Copy summary", "Kopírovat shrnutí"),
    (
        "Copies a short Markdown summary of this week",
        "Zkopíruje krátké shrnutí týdne ve formátu Markdown",
    ),
    ("Summary copied.", "Shrnutí bylo zkopírováno."),
    (
        "This week so far, including meetings still scheduled.",
        "Dosavadní průběh týdne včetně schůzek, které teprve proběhnou.",
    ),
    (
        "The numbers may be incomplete. Days that are not synced are never counted as free.",
        "Čísla nemusí být úplná. Nenačtené dny se nikdy nepočítají jako volné.",
    ),
    (
        "Fewer than 4 of 5 workdays are synced, so this week is not used for comparisons or the weekly notification.",
        "Načteny jsou méně než 4 z 5 pracovních dnů, proto se tento týden nepoužije pro srovnání ani pro týdenní oznámení.",
    ),
    ("Meetings attended", "Vaše schůzky"),
    ("Meeting time", "Čas na schůzkách"),
    ("Overlaps count once", "Překryvy se počítají jednou"),
    ("Per workday", "Na pracovní den"),
    ("Monday to Friday", "Pondělí až pátek"),
    ("Of working time", "Z pracovní doby"),
    ("Mon–Fri 08:00–18:00", "Po–Pá 8:00–18:00"),
    ("Longest free block", "Nejdelší volný blok"),
    (
        "No earlier weeks with enough synced days to compare with.",
        "Pro srovnání chybí předchozí týdny s dostatkem načtených dnů.",
    ),
    ("Time by category", "Čas podle kategorií"),
    ("No meetings this week.", "Tento týden žádné schůzky."),
    (
        "Each meeting counts toward its first category in full, so overlapping meetings can add up to more than the meeting time.",
        "Každá schůzka se celá započítá do své první kategorie, takže překrývající se schůzky mohou dohromady dát víc než čas na schůzkách.",
    ),
    ("Uncategorized", "Bez kategorie"),
    ("No conflicts this week.", "Tento týden žádné konflikty."),
    ("Meetings without an agenda", "Schůzky bez agendy"),
    (
        "No meetings with other participants this week.",
        "Tento týden žádné schůzky s dalšími účastníky.",
    ),
    (
        "Meeting descriptions arrive with the next calendar refresh; agendas are checked after that.",
        "Popisy schůzek přibudou při příští aktualizaci kalendáře; agenda se ověří potom.",
    ),
    (
        "Every checked meeting has an agenda.",
        "Všechny ověřené schůzky mají agendu.",
    ),
    ("You organize", "Pořádáte vy"),
    (
        "Add an agenda so participants can prepare.",
        "Doplňte agendu, ať se účastníci mohou připravit.",
    ),
    ("Others organize", "Pořádají jiní"),
    (
        "Consider asking the organizer for an agenda.",
        "Zvažte, zda organizátora nepožádat o agendu.",
    ),
    (
        "An agenda is any description beyond the Teams join block, links and boilerplate (at least 20 letters or digits).",
        "Agendou je jakýkoli popis kromě bloku pro připojení k Teams, odkazů a šablonového textu (alespoň 20 písmen nebo číslic).",
    ),
    ("Meeting load", "Zatížení schůzkami"),
    (
        "Back-to-back chains (3+ meetings, under 10 min apart)",
        "Schůzky bez přestávky (3 a více, rozestup pod 10 min)",
    ),
    ("Longest chain", "Nejdelší řada"),
    (
        "Days with more than 6 h of meetings",
        "Dny s více než 6 h schůzek",
    ),
    ("Short meetings (30 min or less)", "Krátké schůzky (nejvýše 30 min)"),
    ("Long meetings (90 min or more)", "Dlouhé schůzky (alespoň 90 min)"),
    ("People and customers", "Lidé a zákazníci"),
    (
        "No shared meetings with other people this week.",
        "Tento týden žádné společné schůzky s dalšími lidmi.",
    ),
    ("Most time together", "Nejvíc společného času"),
    ("External companies", "Externí firmy"),
    (
        "No meetings with external companies this week.",
        "Tento týden žádné schůzky s externími firmami.",
    ),
    (
        "Shared meeting time; you, your other mailboxes and rooms are excluded.",
        "Společný čas na schůzkách; vy, vaše další schránky a místnosti se nepočítají.",
    ),
    ("Preparation and follow-through", "Příprava a dotažení"),
    (
        "Notes for ended Teams meetings",
        "Zápisy ze skončených schůzek v Teams",
    ),
    ("Tasks created this week", "Úkoly vytvořené tento týden"),
    ("Tasks completed this week", "Úkoly splněné tento týden"),
    ("AI summary", "Shrnutí od AI"),
    ("Highlights", "Hlavní body"),
    ("Decisions", "Rozhodnutí"),
    ("Open loops", "Nedotažené body"),
    ("Suggestions", "Doporučení"),
    ("Summarizing the week...", "Shrnuji týden…"),
    ("Summarize the week with AI", "Shrnout týden pomocí AI"),
    (
        "One Quick AI request with this week's numbers and saved meeting notes; it counts toward the daily AI limit.",
        "Jeden rychlý požadavek na AI s čísly tohoto týdne a uloženými zápisy ze schůzek; započítá se do denního limitu AI.",
    ),
    (
        "Connect WorkIQ to your calendar account first.",
        "Nejdřív připojte WorkIQ ke svému kalendářovému účtu.",
    ),
    (
        "Never automatic. Only the computed numbers and this week's saved meeting notes are sent; the summary is kept on this computer.",
        "Nikdy se nespouští automaticky. Odesílají se jen spočítaná čísla a uložené zápisy ze schůzek tohoto týdne; shrnutí zůstává v tomto počítači.",
    ),
    (
        "Weekly summary cancelled; nothing was saved.",
        "Shrnutí týdne bylo zrušeno; nic nebylo uloženo.",
    ),
    (
        "Daily AI request limit reached; no weekly summary was requested.",
        "Byl dosažen denní limit požadavků na AI; shrnutí týdne nebylo vyžádáno.",
    ),
    (
        "AI returned an invalid weekly summary; nothing was saved.",
        "AI vrátila neplatné shrnutí týdne; nic nebylo uloženo.",
    ),
    (
        "AI returned an oversized weekly summary; nothing was saved.",
        "AI vrátila příliš dlouhé shrnutí týdne; nic nebylo uloženo.",
    ),
    (
        "AI returned an empty weekly summary; nothing was saved.",
        "AI vrátila prázdné shrnutí týdne; nic nebylo uloženo.",
    ),
    (
        "AI returned a weekly summary outside its limits; nothing was saved.",
        "AI vrátila shrnutí týdne mimo povolené limity; nic nebylo uloženo.",
    ),
    (
        "The weekly summary took too long; try again",
        "Shrnutí týdne trvalo příliš dlouho; zkuste to znovu",
    ),
    (
        "Copilot reported a session error; no weekly summary was produced.",
        "Copilot ohlásil chybu relace; shrnutí týdne nevzniklo.",
    ),
    (
        "Cancelled before the weekly summary was requested",
        "Zrušeno před vyžádáním shrnutí týdne",
    ),
    (
        "The weekly review contains an app credential; nothing was submitted",
        "Týdenní přehled obsahuje přihlašovací údaj aplikace; nic nebylo odesláno",
    ),
    (
        "The weekly review exceeds the AI summary input limit; nothing was submitted",
        "Týdenní přehled překračuje vstupní limit shrnutí od AI; nic nebylo odesláno",
    ),
    ("Saved weekly summary is invalid", "Uložené shrnutí týdne je neplatné"),
    (
        "Saved weekly summary does not match its week",
        "Uložené shrnutí neodpovídá svému týdnu",
    ),
    (
        "Saved weekly summary exceeds its safety limit",
        "Uložené shrnutí týdne překračuje bezpečnostní limit",
    ),
    (
        "Weekly summary exceeds its safety limit",
        "Shrnutí týdne překračuje bezpečnostní limit",
    ),
    ("Read weekly summaries failed", "Načtení shrnutí týdnů se nezdařilo"),
    ("Encode weekly summary failed", "Zakódování shrnutí týdne se nezdařilo"),
    (
        "Begin weekly summary storage failed",
        "Zahájení ukládání shrnutí týdne se nezdařilo",
    ),
    ("Save weekly summary failed", "Uložení shrnutí týdne se nezdařilo"),
    ("Commit weekly summary failed", "Dokončení uložení shrnutí týdne se nezdařilo"),
    (
        "Show a weekly review notification",
        "Zobrazovat oznámení s týdenním přehledem",
    ),
    ("Friday (this week)", "Pátek (tento týden)"),
    ("Monday (previous week)", "Pondělí (minulý týden)"),
    (
        "Once a week with the number of meetings, meeting time, conflicts and meetings without an agenda. If Tomlook is not running at that time, it appears later the same day. It stays quiet unless at least 4 of the week's 5 workdays are synced.",
        "Jednou týdně s počtem schůzek, časem na schůzkách, konflikty a schůzkami bez agendy. Pokud Tomlook v tu dobu neběží, ukáže se později téhož dne. Neozve se, pokud nejsou načteny alespoň 4 z 5 pracovních dnů týdne.",
    ),
    (
        "The weekly review notification does not run in the offline demo; the test uses the demo's fictional calendar.",
        "V offline ukázce se oznámení s týdenním přehledem nespouští; test použije smyšlený kalendář ukázky.",
    ),
    ("Weekly review is turned off.", "Týdenní přehled je vypnutý."),
    (
        "Shows the weekly review notification with the current counts.",
        "Zobrazí oznámení s týdenním přehledem s aktuálními počty.",
    ),
    (
        "Read weekly review history failed",
        "Načtení historie týdenního přehledu se nezdařilo",
    ),
    (
        "Saved weekly review history is invalid",
        "Uložená historie týdenního přehledu je neplatná",
    ),
    (
        "Save weekly review history failed",
        "Uložení historie týdenního přehledu se nezdařilo",
    ),    ("Find a time", "Najít termín"),
    ("Search several days for times that suit the whole group", "Hledat ve více dnech termíny, které vyhovují celé skupině"),
    ("After the invitation is verified, Tomlook offers to delete the BLOCKERs of this proposal.", "Po ověření pozvánky Tomlook nabídne smazání BLOCKERů tohoto návrhu."),
    ("Proposed time", "Navržený termín"),
    ("Add attendees to find a time.", "Pro nalezení termínu přidejte účastníky."),
    ("Candidates", "Možné termíny"),
    ("Chat topic", "Téma chatu"),
    ("Checking Teams and the directory...", "Ověřuji Teams a adresář…"),
    ("Connect WorkIQ to propose in Teams.", "Pro návrh v Teams připojte WorkIQ."),
    ("Create chat and send", "Vytvořit chat a odeslat"),
    ("Create meeting", "Vytvořit schůzku"),
    ("Daily window", "Denní rozmezí"),
    ("Duration", "Délka"),
    ("Enter a topic and select 1 to 5 times", "Zadejte téma a vyberte 1 až 5 termínů"),
    ("From", "Od"),
    ("Invitees", "Pozvaní"),
    ("Members", "Členové"),
    ("Needs a check", "Vyžaduje kontrolu"),
    ("Next day", "Další den"),
    ("Previous day", "Předchozí den"),
    ("No BLOCKERs of this proposal are in your calendar any more.", "Ve vašem kalendáři už nejsou žádné BLOCKERy tohoto návrhu."),
    ("No free time was found in this range; try more days or a longer daily window.", "V tomto období se nenašel volný čas; zkuste více dní nebo delší denní rozmezí."),
    ("No time works for every required attendee; these are the best alternatives.", "Žádný termín nevyhovuje všem povinným účastníkům; toto jsou nejlepší alternativy."),
    ("Nothing is sent to a reduced group. Use Propose by e-mail instead.", "Zúžené skupině se nic neposílá. Použijte raději návrh e-mailem."),
    ("Open the chat in Teams", "Otevřít chat v Teams"),
    ("Opens the new event with these attendees and this time", "Otevře novou událost s těmito účastníky v tomto čase"),
    ("Recipients", "Příjemci"),
    ("Release all BLOCKERs", "Uvolnit všechny BLOCKERy"),
    ("Scheduling proposals", "Návrhy termínů"),
    ("Select exactly one time", "Vyberte právě jeden termín"),
    ("Select up to 5 times. Unknown availability is never treated as free.", "Vyberte až 5 termínů. Neznámá dostupnost se nikdy nepovažuje za volno."),
    ("Show at least", "Zobrazit alespoň"),
    ("Teams proposals are unavailable", "Návrhy v Teams nejsou dostupné"),
    ("The description lists the topic, the invitees and every proposed time. Nobody is invited.", "Popis obsahuje téma, pozvané a všechny navržené termíny. Nikdo není pozván."),
    ("The invitation was created and verified. Delete the BLOCKERs of this proposal?", "Pozvánka byla vytvořena a ověřena. Smazat BLOCKERy tohoto návrhu?"),
    ("These people cannot be added to a Teams chat:", "Tyto osoby nelze přidat do chatu v Teams:"),
    ("Tomlook never sends e-mail: copy the text or open a new message in your e-mail app.", "Tomlook e-maily nikdy neodesílá: zkopírujte text nebo otevřete novou zprávu v e-mailové aplikaci."),
    ("Topic", "Téma"),
    ("Use Propose by e-mail instead.", "Použijte raději návrh e-mailem."),
    ("Weekdays only", "Jen pracovní dny"),
    ("With one person, Tomlook uses your one-on-one chat, which has no topic.", "S jednou osobou Tomlook použije váš společný chat jen s ní, který nemá téma."),
    ("Your time is protected. Next, propose these times in Teams or by e-mail.", "Váš čas je zablokovaný. Teď termíny navrhněte v Teams nebo e-mailem."),
    ("Confirm this time", "Potvrdit tento termín"),
    ("Find another time with these people", "Najít jiný termín s těmito lidmi"),
    ("Held by Tomlook for", "Blokováno Tomlookem pro"),
    ("Opens Find a time with this meeting's people, title and duration", "Otevře Najít termín s lidmi, názvem a délkou této schůzky"),
    ("Opens the invitation with the proposal's attendees at this time", "Otevře pozvánku s účastníky návrhu v tomto čase"),
    ("Hold with BLOCKERs", "Blokovat termíny"),
    ("Propose in a Teams group chat", "Navrhnout ve skupinovém chatu Teams"),
    ("Propose by e-mail", "Navrhnout e-mailem"),
    ("Workdays", "Pracovní dny"),
    ("Days", "Dny"),
    ("Meeting scheduled", "Schůzka je naplánovaná"),
    ("Close the proposal", "Uzavřít návrh"),
    ("OK", "OK"),
    ("Keep them", "Ponechat"),
    ("BLOCKERs are being created; nothing else is sent.", "BLOCKERy se vytvářejí; nic dalšího se neodesílá."),
    ("Sending the proposal to Teams...", "Odesílám návrh do Teams…"),
    ("The Teams proposal could not be queued; nothing was sent", "Návrh do Teams se nepodařilo zařadit; nic nebylo odesláno"),
    ("The offline demo never changes a calendar or contacts anyone.", "Offline ukázka nikdy nemění kalendář ani nikoho nekontaktuje."),
    ("Open proposal", "Otevřený návrh"),
    ("Scheduled", "Naplánováno"),
    ("Creating BLOCKER", "Vytvářím BLOCKER"),
    ("BLOCKER in your calendar", "BLOCKER je ve vašem kalendáři"),
    ("BLOCKER not created", "BLOCKER nebyl vytvořen"),
    ("Check this BLOCKER in Outlook", "Zkontrolujte tento BLOCKER v Outlooku"),
    ("Deleting BLOCKER", "Mažu BLOCKER"),
    ("BLOCKER deleted", "BLOCKER je smazaný"),
    ("Check whether this BLOCKER was deleted in Outlook", "Zkontrolujte v Outlooku, zda byl tento BLOCKER smazán"),
    ("Sending to Teams", "Odesílám do Teams"),
    ("Sent in Teams", "Odesláno v Teams"),
    ("Not sent in Teams", "V Teams neodesláno"),
    ("Check the Teams chat", "Zkontrolujte chat v Teams"),
    ("Creating BLOCKERs", "Vytvářím BLOCKERy"),
    ("BLOCKERs created", "BLOCKERy jsou vytvořené"),
    ("BLOCKERs not created", "BLOCKERy nebyly vytvořeny"),
    ("Check BLOCKERs in Outlook", "Zkontrolujte BLOCKERy v Outlooku"),
    ("Enter a topic (up to 200 bytes)", "Zadejte téma (nejvýše 200 bajtů)"),
    ("The meeting must last 15 to 240 minutes in 15-minute steps", "Schůzka musí trvat 15 až 240 minut v krocích po 15 minutách"),
    ("Search 1 to 15 days", "Hledejte v rozmezí 1 až 15 dní"),
    ("The daily window must be longer than the meeting", "Denní rozmezí musí být delší než schůzka"),
    ("Show at least 1 to 10 candidates", "Zobrazte alespoň 1 až 10 termínů"),
    ("Add attendees to find a time", "Pro nalezení termínu přidejte účastníky"),
    ("Select at most 5 times", "Vyberte nejvýše 5 termínů"),
    ("Select 1 to 5 times", "Vyberte 1 až 5 termínů"),
    ("Proposed times must be distinct and last the meeting's duration", "Navržené termíny se nesmí opakovat a musí trvat celou délku schůzky"),
    ("Write a message of up to 4000 bytes", "Napište zprávu o délce nejvýše 4000 bajtů"),
    ("Every Teams member must be one resolved person", "Každý člen chatu v Teams musí být jedna dohledaná osoba"),
    ("A Teams proposal needs 1 to 50 people", "Návrh v Teams potřebuje 1 až 50 osob"),
    ("One-on-one chats have no topic", "Chat jen se dvěma lidmi nemá téma"),
    ("Group chats need a topic", "Skupinový chat potřebuje téma"),
    ("Saved BLOCKERs do not match their proposal", "Uložené BLOCKERy neodpovídají svému návrhu"),
    ("Saved Teams proposal is invalid", "Uložený návrh v Teams je neplatný"),
    ("Invalid proposal identity", "Neplatná identita návrhu"),
    ("Your account cannot be added to the chat", "Váš účet nelze přidat do chatu"),
    ("BLOCKERs were already requested for this proposal", "BLOCKERy už byly pro tento návrh vyžádány"),
    ("BLOCKER approval does not match its proposal", "Schválení BLOCKERů neodpovídá svému návrhu"),
    ("Wait for the current proposal action to finish", "Počkejte na dokončení aktuální akce návrhu"),
    ("This BLOCKER is no longer held by the proposal", "Tento BLOCKER už k návrhu nepatří"),
    ("This proposal was already sent to Teams; open the chat instead", "Tento návrh už byl odeslán do Teams; otevřete raději chat"),
    ("Teams members must be the proposal's attendees", "Členové chatu v Teams musí být účastníci návrhu"),
    ("This proposal was cancelled", "Tento návrh byl zrušen"),
    ("Not attempted after an earlier BLOCKER failed", "Nezkoušeno, protože předchozí BLOCKER selhal"),
    ("The Teams outcome is uncertain. Open Teams before proposing again; nothing will be sent again automatically.", "Výsledek v Teams je nejistý. Před dalším návrhem otevřete Teams; nic se znovu automaticky neodešle."),
    ("The deletion outcome is uncertain. Check Outlook; nothing will be deleted again automatically.", "Výsledek mazání je nejistý. Zkontrolujte Outlook; nic se znovu automaticky nesmaže."),
    ("Teams chats cannot be created through your WorkIQ connection; propose by e-mail instead", "Chaty v Teams nelze přes vaše připojení WorkIQ vytvářet; navrhněte termíny raději e-mailem"),
    ("Directory lookups are not permitted for this account; propose by e-mail instead", "Hledání v adresáři není pro tento účet povoleno; navrhněte termíny raději e-mailem"),
    ("The Teams chat was created, but its members could not be verified, so no message was sent. Open the chat in Teams.", "Chat v Teams byl vytvořen, ale jeho členy se nepodařilo ověřit, proto nebyla odeslána žádná zpráva. Otevřete chat v Teams."),
    ("The Teams chat does not have exactly the approved members, so no message was sent. Open the chat in Teams.", "Chat v Teams nemá přesně schválené členy, proto nebyla odeslána žádná zpráva. Otevřete chat v Teams."),
    ("The message may have been sent. Open the Teams chat before sending anything again.", "Zpráva možná byla odeslána. Než cokoli pošlete znovu, otevřete chat v Teams."),
    ("Teams chat created and the message was read back", "Chat v Teams byl vytvořen a zpráva byla ověřena zpětným načtením"),
    ("The message was sent but could not be read back. Open the Teams chat to check it.", "Zpráva byla odeslána, ale nepodařilo se ji zpětně načíst. Zkontrolujte ji v chatu v Teams."),
    ("Teams denied creating the chat; nothing was sent.", "Teams vytvoření chatu odmítl; nic nebylo odesláno."),
    ("Teams denied the message; the chat exists without it.", "Teams zprávu odmítl; chat existuje bez ní."),
    ("The approved Teams request is invalid; nothing was sent.", "Schválený požadavek pro Teams je neplatný; nic nebylo odesláno."),
    ("Invalid Teams chat identity", "Neplatná identita chatu v Teams"),
    ("Teams members are unavailable", "Členové chatu v Teams nejsou dostupní"),
    ("The Teams chat has more members than were approved", "Chat v Teams má více členů, než bylo schváleno"),
    ("A Teams member has no verifiable identity", "Člen chatu v Teams nemá ověřitelnou identitu"),
    ("Invalid Teams message identity", "Neplatná identita zprávy v Teams"),
    ("The same person is listed twice", "Stejná osoba je uvedena dvakrát"),
    ("Teams schema discovery timed out", "Zjištění schématu Teams vypršelo"),
    ("Teams schema discovery failed; nothing was sent", "Zjištění schématu Teams se nezdařilo; nic nebylo odesláno"),
    ("BLOCKER deleted and verified", "BLOCKER je smazaný a ověřený"),
    ("Already deleted in Outlook", "Už smazáno v Outlooku"),
    ("This event is no longer the BLOCKER Tomlook created; nothing was deleted. Delete it in Outlook if needed.", "Tato událost už není BLOCKER vytvořený Tomlookem; nic nebylo smazáno. V případě potřeby ji smažte v Outlooku."),
    ("The BLOCKER changed before deletion; nothing was deleted", "BLOCKER se před smazáním změnil; nic nebylo smazáno"),
    ("Calendar account changed; nothing was deleted", "Účet kalendáře se změnil; nic nebylo smazáno"),
    ("A proposed time is already in the past; find a time again", "Navržený termín už je v minulosti; najděte termín znovu"),
    ("This proposal changed; find a time again", "Tento návrh se změnil; najděte termín znovu"),
    ("A new proposal cannot carry earlier outcomes", "Nový návrh nemůže nést dřívější výsledky"),
    ("This proposal no longer exists", "Tento návrh už neexistuje"),
    ("Choose the BLOCKERs to delete", "Vyberte BLOCKERy ke smazání"),
    ("The meeting was not created and verified", "Schůzka nebyla vytvořena a ověřena"),
    ("You are added to the chat automatically; remove yourself from the members", "Do chatu jste přidáni automaticky; odeberte sami sebe ze členů"),
    ("Connect WorkIQ before sending a Teams proposal", "Před odesláním návrhu do Teams připojte WorkIQ"),
    ("The offline demo never contacts anyone", "Offline ukázka nikdy nikoho nekontaktuje"),
    ("Tomlook is stopping; nothing was sent", "Tomlook se ukončuje; nic nebylo odesláno"),
    ("Tomlook is stopping; nothing was deleted", "Tomlook se ukončuje; nic nebylo smazáno"),
    ("The proposal could not be saved; nothing was submitted", "Návrh se nepodařilo uložit; nic nebylo odesláno"),
    ("The Teams proposal was not queued; nothing was sent", "Návrh do Teams nebyl zařazen; nic nebylo odesláno"),
    ("The deletion was not queued; nothing was deleted", "Smazání nebylo zařazeno; nic nebylo smazáno"),
    ("Teams writes are unavailable or busy; nothing was sent", "Zápis do Teams není dostupný nebo je obsazený; nic nebylo odesláno"),
    ("Calendar writes are unavailable or busy; nothing was deleted", "Zápis do kalendáře není dostupný nebo je obsazený; nic nebylo smazáno"),
    ("Wait for the current calendar change to finish", "Počkejte na dokončení aktuální změny kalendáře"),
    ("Preview never changes a live calendar", "Náhled nikdy nemění skutečný kalendář"),
    ("Calendar account changed before confirmation", "Účet kalendáře se před potvrzením změnil"),
    ("This BLOCKER's time cannot be edited here; schedule the meeting in Outlook", "Čas tohoto BLOCKERu tu nelze upravit; naplánujte schůzku v Outlooku"),
    ("The Teams check could not be queued; try again", "Ověření Teams se nepodařilo zařadit; zkuste to znovu"),
    ("Connect WorkIQ to propose in Teams", "Pro návrh v Teams připojte WorkIQ"),
    ("Your own availability is unknown; connect WorkIQ again", "Vaše vlastní dostupnost není známá; připojte znovu WorkIQ"),
    ("Availability batches cover different ranges", "Části odpovědi o dostupnosti pokrývají různá období"),
    ("Customers", "Zákazníci"),
    ("Customers in this meeting", "Zákazníci na této schůzce"),
    ("Add to customer", "Přidat k zákazníkovi"),
    ("All customers", "Všichni zákazníci"),
    ("Search customers...", "Hledat zákazníky..."),
    ("No matching customers", "Žádní odpovídající zákazníci"),
    ("No confirmed customers yet. Confirm one from a meeting with external attendees.", "Zatím nemáte žádné potvrzené zákazníky. Potvrďte je ze schůzky s externími účastníky."),
    ("Unconfirmed customers in your meetings", "Nepotvrzení zákazníci ve vašich schůzkách"),
    ("Confirm customer", "Potvrdit zákazníka"),
    ("Edit customer", "Upravit zákazníka"),
    ("Open customer", "Otevřít zákazníka"),
    ("Open customers", "Otevřít zákazníky"),
    ("Customer timeline", "Časová osa zákazníka"),
    ("Customer name", "Název zákazníka"),
    ("E-mail domain", "E-mailová doména"),
    ("E-mail domains", "E-mailové domény"),
    ("People in this meeting", "Lidé z této schůzky"),
    ("New customer", "Nový zákazník"),
    ("MSX customer", "Zákazník v MSX"),
    ("Local customer only (no MSX link)", "Jen místní zákazník (bez propojení s MSX)"),
    ("Local customer without an MSX link", "Místní zákazník bez propojení s MSX"),
    ("MSX is not connected; you can confirm a local customer now and link MSX later.", "MSX není připojené. Zákazníka teď můžete potvrdit jako místního a s MSX ho propojit později."),
    ("Search MSX", "Hledat v MSX"),
    ("Search MSX again", "Hledat v MSX znovu"),
    ("Searching MSX for matching customers...", "Hledám v MSX odpovídající zákazníky..."),
    ("No MSX candidates found. Confirm a local customer or search again later.", "V MSX se nenašel žádný kandidát. Potvrďte místního zákazníka, nebo hledejte později znovu."),
    ("Several candidates are possible; choose the right one.", "Připadá v úvahu víc kandidátů; vyberte správného."),
    ("Likely match", "Pravděpodobná shoda"),
    ("Inferred matches are never linked without your confirmation.", "Odvozené shody se nikdy nepropojí bez vašeho potvrzení."),
    ("Confirm", "Potvrdit"),
    ("Not a customer meeting", "Není to schůzka se zákazníkem"),
    ("Only this meeting; other meetings with these people are unchanged.", "Jen tato schůzka; ostatní schůzky s těmito lidmi se nemění."),
    ("Marked as not a customer meeting.", "Označeno jako schůzka bez zákazníka."),
    ("Detect automatically", "Rozpoznat automaticky"),
    ("Set for this meeting", "Nastaveno pro tuto schůzku"),
    ("Only this customer", "Jen tento zákazník"),
    ("Sample data", "Ukázková data"),
    ("Choose an MSX customer or keep a local one; nothing is linked without you.", "Vyberte zákazníka z MSX, nebo ponechte místního; bez vás se nic nepropojí."),
    ("Open in MSX", "Otevřít v MSX"),
    ("MSX not read yet", "Z MSX zatím nic nenačteno"),
    ("Loading MSX...", "Načítám MSX..."),
    ("Reads opportunities, milestones, your MSX tasks and Azure consumption again (read-only).", "Znovu načte příležitosti, milníky, vaše úkoly v MSX a spotřebu Azure (jen pro čtení)."),
    ("Unlink MSX", "Zrušit propojení s MSX"),
    ("Remove customer", "Odebrat zákazníka"),
    ("Remove this customer with its domains and meeting choices? Meetings, notes and tasks stay.", "Odebrat tohoto zákazníka i s jeho doménami a volbami u schůzek? Schůzky, poznámky a úkoly zůstanou."),
    ("Sample data: fictional, not read from MSX or produced by AI.", "Ukázková data: smyšlená, nenačtená z MSX a nevytvořená AI."),
    ("Meetings with this customer", "Schůzky s tímto zákazníkem"),
    ("Meetings in the last 90 days", "Schůzky za posledních 90 dní"),
    ("Hours together in the last 90 days", "Společné hodiny za posledních 90 dní"),
    ("None scheduled", "Žádná naplánovaná"),
    ("No meetings with this customer in the local calendar.", "V místním kalendáři nejsou žádné schůzky s tímto zákazníkem."),
    ("Earlier meetings", "Dřívější schůzky"),
    ("From the local calendar cache: one month back and six months ahead.", "Z místně uloženého kalendáře: měsíc zpět a šest měsíců dopředu."),
    ("Notes and decisions", "Poznámky a rozhodnutí"),
    ("No saved meeting notes for this customer yet.", "K tomuto zákazníkovi zatím nejsou uložené žádné poznámky ze schůzek."),
    ("Discussed", "Probíralo se"),
    ("Tasks from these meetings", "Úkoly z těchto schůzek"),
    ("No open tasks from these meetings.", "Z těchto schůzek nejsou žádné otevřené úkoly."),
    ("This customer is not linked to MSX.", "Tento zákazník není propojený s MSX."),
    ("Link MSX customer", "Propojit se zákazníkem v MSX"),
    ("Not signed in to MSX. Sign in to read opportunities, milestones and consumption.", "Nejste přihlášeni k MSX. Po přihlášení se načtou příležitosti, milníky a spotřeba."),
    ("A failed read is not evidence that nothing exists.", "Neúspěšné načtení neznamená, že nic neexistuje."),
    ("Partial MSX data:", "Neúplná data z MSX:"),
    ("Open opportunities", "Otevřené příležitosti"),
    ("Estimated close", "Odhad uzavření"),
    ("Active future milestones", "Aktivní budoucí milníky"),
    ("My task: yes", "Můj úkol: ano"),
    ("My task: no", "Můj úkol: ne"),
    ("My task: unknown", "Můj úkol: neznámo"),
    ("No active future milestones in a complete read.", "Úplné načtení nenašlo žádné aktivní budoucí milníky."),
    ("My MSX tasks", "Moje úkoly v MSX"),
    ("The read reached its limit; more may exist.", "Načítání dosáhlo limitu; může existovat víc záznamů."),
    ("None found in a complete read.", "Úplné načtení nic nenašlo."),
    ("Opportunities reached the read limit; more may exist.", "Příležitosti dosáhly limitu načítání; může jich být víc."),
    ("Milestones reached the read limit; more may exist.", "Milníky dosáhly limitu načítání; může jich být víc."),
    ("Your milestone tasks could not be fully checked.", "Vaše úkoly u milníků se nepodařilo úplně ověřit."),
    ("Opportunities and milestones are unavailable.", "Příležitosti a milníky nejsou k dispozici."),
    ("Azure consumption is unavailable.", "Spotřeba Azure není k dispozici."),
    ("Azure consumption", "Spotřeba Azure"),
    ("Link an MSX customer with a TPID to see Azure consumption.", "Spotřebu Azure uvidíte po propojení se zákazníkem v MSX, který má TPID."),
    ("No TPID is linked; Azure consumption needs the customer's TPID.", "Chybí TPID; spotřeba Azure vyžaduje TPID zákazníka."),
    ("Completed month", "Uzavřený měsíc"),
    ("Current month (partial)", "Aktuální měsíc (neúplný)"),
    ("Model prediction", "Předpověď modelu"),
    ("No month has data yet.", "Zatím žádný měsíc s daty."),
    ("Consumption is technical evidence, not business intent.", "Spotřeba je technický údaj, ne obchodní záměr."),
    ("Sample prediction; fictional.", "Ukázková předpověď; smyšlená."),
    ("Prediction omitted: the model exposes no calibrated forecast measure; the current month is a partial actual.", "Předpověď chybí: model nenabízí kalibrovanou míru předpovědi; aktuální měsíc je neúplná skutečnost."),
    ("Recent communication", "Nedávná komunikace"),
    ("Find related communication", "Najít související komunikaci"),
    ("Searching mail, Teams and meetings...", "Prohledávám poštu, Teams a schůzky..."),
    ("One Thorough AI request with read-only mail, Teams and meeting search over the last 60 days; it counts toward the daily AI limit.", "Jeden důkladný požadavek na AI, který jen pro čtení prohledá poštu, Teams a schůzky za posledních 60 dní; započítá se do denního limitu AI."),
    ("Not searched yet. Tomlook searches only when you ask, and before preparing a confirmed customer meeting it reuses the last summary.", "Zatím neprohledáno. Tomlook hledá jen na vaši žádost a při přípravě schůzky s potvrzeným zákazníkem použije poslední souhrn."),
    ("Sample communication summary: fictional, not produced by AI.", "Ukázkový souhrn komunikace: smyšlený, nevytvořila ho AI."),
    ("No related communication found in the searched scope.", "V prohledaném rozsahu se nenašla žádná související komunikace."),
    ("Open source", "Otevřít zdroj"),
    ("Source not among the retrieved results", "Zdroj nebyl mezi načtenými výsledky"),
    ("Coverage gaps", "Co se nepodařilo pokrýt"),
    ("Commitments", "Závazky"),
    ("E-mail", "E-mail"),
    ("Teams", "Teams"),
    ("Link an MSX customer first.", "Nejdřív propojte zákazníka v MSX."),
    ("Sign in to MSX first.", "Nejdřív se přihlaste k MSX."),
    ("Connect GitHub Copilot and WorkIQ first.", "Nejdřív připojte GitHub Copilot a WorkIQ."),
    ("Another communication search is running.", "Právě běží jiné hledání komunikace."),
    ("Communication search cancelled; nothing was saved.", "Hledání komunikace bylo zrušeno; nic se neuložilo."),
    ("The communication summary belongs to a different customer; nothing was saved.", "Souhrn komunikace patří jinému zákazníkovi; nic se neuložilo."),
    ("Daily AI request limit reached; no communication research was requested.", "Denní limit požadavků na AI je vyčerpaný; hledání komunikace nebylo odesláno."),
    ("Connect WorkIQ with this calendar's account before searching communication; nothing was submitted.", "Před hledáním komunikace připojte WorkIQ s účtem tohoto kalendáře; nic nebylo odesláno."),
    ("AI returned an invalid communication summary; nothing was saved.", "AI vrátila neplatný souhrn komunikace; nic se neuložilo."),
    ("AI returned more communication than allowed; nothing was saved.", "AI vrátila víc komunikace, než je povoleno; nic se neuložilo."),
    ("AI returned an unknown kind of communication; nothing was saved.", "AI vrátila neznámý druh komunikace; nic se neuložilo."),
    ("AI returned an unknown commitment owner; nothing was saved.", "AI vrátila neznámého nositele závazku; nic se neuložilo."),
    ("AI returned an oversized communication summary; nothing was saved.", "AI vrátila příliš dlouhý souhrn komunikace; nic se neuložilo."),
    ("AI returned an empty communication summary; nothing was saved.", "AI vrátila prázdný souhrn komunikace; nic se neuložilo."),
    ("Communication research took too long; try again", "Hledání komunikace trvalo příliš dlouho; zkuste to znovu"),
    ("Communication research cancelled; nothing was saved.", "Hledání komunikace bylo zrušeno; nic se neuložilo."),
    ("Copilot reported a session error; no communication summary was produced.", "Copilot nahlásil chybu relace; souhrn komunikace nevznikl."),
    ("Add an e-mail domain to this customer before searching communication", "Před hledáním komunikace přidejte zákazníkovi e-mailovou doménu"),
    ("This customer belongs to a different calendar account", "Tento zákazník patří k jinému účtu kalendáře"),
    ("The customer research request exceeds its input limit; nothing was submitted", "Požadavek na hledání komunikace se zákazníkem překračuje limit vstupu; nic nebylo odesláno"),
    ("MSX is not connected. Sign in from the MSX status first; nothing was read.", "MSX není připojené. Nejdřív se přihlaste přes stav MSX; nic nebylo načteno."),
    ("MSX did not answer within 150 seconds", "MSX neodpovědělo do 150 sekund"),
    ("MSX did not answer within 120 seconds", "MSX neodpovědělo do 120 sekund"),
    ("Enter e-mail domains such as contoso.com", "Zadejte e-mailové domény, například contoso.com"),
    ("A consumer mail domain cannot identify a customer", "Doména veřejné pošty nemůže určit zákazníka"),
    ("A customer can have at most 20 domains", "Zákazník může mít nejvýš 20 domén"),
    ("Customer name must have 1-120 characters on one line", "Název zákazníka musí mít 1–120 znaků na jednom řádku"),
    ("This customer no longer exists", "Tento zákazník už neexistuje"),
    ("Search MSX again before linking a customer", "Před propojením se zákazníkem v MSX hledejte znovu"),
    ("At most 1,000 customers can be saved", "Uložit lze nejvýš 1 000 zákazníků"),
    ("Saved customer identity is invalid", "Uložená identita zákazníka je neplatná"),
    ("Customer domains are invalid", "Domény zákazníka jsou neplatné"),
    ("The linked MSX account is invalid", "Propojený zákazník v MSX je neplatný"),
    ("Add customer context to meeting preparation", "Přidávat do přípravy schůzek kontext zákazníka"),
    ("For confirmed customer meetings, preparation adds the customer, open opportunities and next milestones, the Azure consumption trend (when MSX is connected) and the last communication summary. It is read when a meeting is prepared and never prepares existing Insights again by itself.", "U schůzek s potvrzeným zákazníkem příprava přidá zákazníka, otevřené příležitosti a nejbližší milníky, vývoj spotřeby Azure (je-li připojené MSX) a poslední souhrn komunikace. Načítá se až při přípravě schůzky a sama od sebe nikdy znovu nepřipravuje existující přehledy."),
    ("Setup", "Nastavení připojení"),
    ("Setup checklist", "Kontrolní seznam nastavení"),
    ("Welcome", "Vítejte"),
    ("Welcome to Tomlook", "Vítejte v Tomlooku"),
    (
        "Tomlook is a fast Windows calendar for your Microsoft 365 meetings. With GitHub Copilot it prepares Insights before meetings and notes after them. Nothing connects until you sign in.",
        "Tomlook je rychlý kalendář pro Windows na vaše schůzky v Microsoft 365. S GitHub Copilotem připraví přehledy před schůzkami a poznámky po nich. Nic se nepřipojí, dokud se nepřihlásíte.",
    ),
    ("Open setup checklist", "Otevřít kontrolní seznam"),
    (
        "The checklist stays in Settings > Setup and in Commands (Ctrl+Shift+P).",
        "Kontrolní seznam najdete v Nastavení > Nastavení připojení a v příkazech (Ctrl+Shift+P).",
    ),
    (
        "What is connected, what is missing and what to do next. Nothing signs in until you click.",
        "Co je připojené, co chybí a co udělat dál. Nic se nepřihlásí, dokud na to nekliknete.",
    ),
    (
        "Offline demo: Tomlook never connects to Microsoft 365, GitHub Copilot or other services.",
        "Offline ukázka: Tomlook se nikdy nepřipojuje k Microsoft 365, GitHub Copilotu ani jiným službám.",
    ),
    ("All required steps are done.", "Všechny povinné kroky jsou hotové."),
    (
        "Looks for Azure CLI and the Copilot runtime again and rereads the Windows notification setting. No sign-in or network request.",
        "Znovu vyhledá Azure CLI a runtime Copilotu a znovu načte nastavení oznámení ve Windows. Nic se nepřihlašuje a nic se neodesílá do sítě.",
    ),
    ("Show setup progress in the status bar", "Zobrazovat postup nastavení ve stavovém řádku"),
    ("Required steps", "Povinné kroky"),
    ("Optional connections", "Nepovinná připojení"),
    ("Prerequisites", "Požadavky"),
    ("Details", "Podrobnosti"),
    ("Copilot CLI installation guide", "Návod k instalaci Copilot CLI"),
    (
        "Open the setup checklist. Right-click to hide this indicator.",
        "Otevře kontrolní seznam nastavení. Kliknutím pravým tlačítkem tento ukazatel skryjete.",
    ),
    ("Don't show", "Nezobrazovat"),
    ("Microsoft 365 calendar (WorkIQ)", "Kalendář Microsoft 365 (WorkIQ)"),
    ("GitHub Copilot", "GitHub Copilot"),
    ("Identity match", "Shoda účtů"),
    ("Windows notifications", "Oznámení Windows"),
    ("WebIQ", "WebIQ"),
    ("GitHub connector", "Konektor GitHub"),
    ("Foundry / Auto mode", "Foundry / režim Auto"),
    ("Runtime prerequisites", "Požadavky na počítač"),
    (
        "Loads your calendar, people and availability; keeps one month back and six months ahead in sync.",
        "Načítá váš kalendář, lidi a jejich dostupnost; synchronizuje měsíc zpět a šest měsíců dopředu.",
    ),
    (
        "Creates Insights, answers and meeting notes. Live AI needs an installed GitHub Copilot CLI runtime.",
        "Vytváří přehledy, odpovědi a poznámky ze schůzek. Živá AI potřebuje nainstalovaný runtime GitHub Copilot CLI.",
    ),
    (
        "Automatic Insights require matching connected identities and verified calendar coverage.",
        "Automatické přehledy vyžadují shodné připojené účty a ověřené pokrytí kalendáře.",
    ),
    (
        "Meeting reminders, My day and the weekly review appear as Windows notifications.",
        "Připomínky schůzek, Můj den a týdenní přehled se zobrazují jako oznámení Windows.",
    ),
    (
        "Checks GitHub Releases once a day and installs only signed Tomlook releases.",
        "Jednou denně zkontroluje GitHub Releases a nainstaluje jen podepsaná vydání Tomlooku.",
    ),
    (
        "Public web search for meeting and customer research.",
        "Veřejné hledání na webu při přípravě schůzek a zjišťování o zákaznících.",
    ),
    (
        "Read-only access to selected repositories for research.",
        "Přístup jen pro čtení k vybraným repozitářům při zjišťování souvislostí.",
    ),
    (
        "Microsoft-Decision-1 chooses Quick or Thorough in Auto mode, using your Azure CLI sign-in.",
        "Microsoft-Decision-1 v režimu Auto vybírá Rychle nebo Důkladně a používá vaše přihlášení v Azure CLI.",
    ),
    ("What Tomlook itself needs on this computer.", "Co Tomlook potřebuje na tomto počítači."),
    ("Action needed", "Vyžaduje akci"),
    ("Not set up", "Nenastaveno"),
    ("Error", "Chyba"),
    ("Open WorkIQ connection", "Otevřít připojení WorkIQ"),
    ("Retry sync", "Zkusit synchronizaci znovu"),
    ("Open AI connection", "Otevřít připojení AI"),
    ("Install Copilot CLI", "Nainstalovat Copilot CLI"),
    ("Open Insights activity", "Otevřít průběh zjišťování"),
    ("Turn on automatic checks", "Zapnout automatickou kontrolu"),
    ("Enter key", "Zadat klíč"),
    ("Connect", "Připojit"),
    ("Open Foundry connection", "Otevřít připojení Foundry"),
    ("Open MSX settings", "Otevřít nastavení MSX"),
    ("Check again", "Zkontrolovat znovu"),
    ("Status", "Stav"),
    ("Signed-in account", "Přihlášený účet"),
    ("Calendar owner", "Vlastník kalendáře"),
    ("Calendar sync", "Synchronizace kalendáře"),
    ("Last saved refresh", "Poslední uložené obnovení"),
    ("Copilot runtime", "Runtime Copilotu"),
    ("WorkIQ account", "Účet WorkIQ"),
    ("Reason", "Důvod"),
    ("Last test notification", "Poslední zkušební oznámení"),
    ("Last check", "Poslední kontrola"),
    ("Latest release", "Nejnovější vydání"),
    ("Azure CLI", "Azure CLI"),
    ("Visual C++ runtime", "Runtime Visual C++"),
    ("Not synced yet", "Zatím nesynchronizováno"),
    ("Not found", "Nenalezeno"),
    ("Available (Tomlook is running)", "K dispozici (Tomlook běží)"),
    ("Not available in the offline demo.", "V offline ukázce není k dispozici."),
    (
        "Calendar sync failed; retrying automatically",
        "Synchronizace kalendáře selhala; Tomlook ji automaticky zkusí znovu",
    ),
    (
        "Synced: 1 month back – 6 months ahead",
        "Synchronizováno: měsíc zpět – 6 měsíců dopředu",
    ),
    (
        "Signed in to the isolated Copilot runtime",
        "Přihlášeno do izolovaného runtime Copilotu",
    ),
    ("Copilot runtime is not set up", "Runtime Copilotu není nastavený"),
    ("Not signed in", "Nepřihlášeno"),
    ("Automatic Insights can run", "Automatické přehledy mohou běžet"),
    ("Automatic Insights cannot run yet", "Automatické přehledy zatím nemohou běžet"),
    ("Notifications are unavailable", "Oznámení nejsou k dispozici"),
    ("Windows reported a notification error", "Windows nahlásil chybu oznámení"),
    (
        "Checking the Windows notification setting...",
        "Ověřuji nastavení oznámení ve Windows…",
    ),
    ("The last update check failed", "Poslední kontrola aktualizací selhala"),
    ("Automatic update checks are on", "Automatická kontrola aktualizací je zapnutá"),
    ("Automatic update checks are off", "Automatická kontrola aktualizací je vypnutá"),
    ("Connection failed", "Připojení selhalo"),
    ("Signed in", "Přihlášeno"),
    ("Not configured", "Nenakonfigurováno"),
    ("Checking prerequisites...", "Ověřuji požadavky…"),
    ("All prerequisites found", "Všechny požadavky jsou splněné"),
    (
        "Azure CLI not found (needed only for Foundry and MSX)",
        "Azure CLI nebylo nalezeno (potřebují ho jen Foundry a MSX)",
    ),
    (
        "Copilot runtime is not set up and Azure CLI was not found",
        "Runtime Copilotu není nastavený a Azure CLI nebylo nalezeno",
    ),
    ("Automatic Insights are turned off.", "Automatické přehledy jsou vypnuté."),
    ("WorkIQ is not connected.", "WorkIQ není připojené."),
    (
        "The connected WorkIQ account does not own this calendar. Refresh with the correct account before retrying.",
        "Připojený účet WorkIQ není vlastníkem tohoto kalendáře. Než to zkusíte znovu, obnovte kalendář se správným účtem.",
    ),
    (
        "Connect WorkIQ before preparing this meeting",
        "Před přípravou této schůzky připojte WorkIQ",
    ),
    (
        "Prerequisite check stopped unexpectedly",
        "Kontrola požadavků se neočekávaně zastavila",
    ),
];

const PATTERNS: &[(&str, &str)] = &[
    ("Enter {} at github.com/login/device", "Zadejte kód {0} na github.com/login/device"),
    ("AI sign-in failed: {}", "Přihlášení k AI selhalo: {0}"),
    (
        "Prompt suggestion exceeded its {}-second limit; retry explicitly",
        "Návrh promptu překročil limit {0} s; zkuste to znovu ručně.",
    ),
    (
        "Automatic reconnect in about {} seconds.",
        "Další automatický pokus přibližně za {0} s.",
    ),
    (
        "Microsoft-Decision-1 returned HTTP {}; choose a mode explicitly",
        "Microsoft-Decision-1 vrátil HTTP {0}; vyberte režim ručně.",
    ),
    ("{}: Retrieved", "{0}: Načteno"),
    ("{}: Retrieval failed", "{0}: Načtení se nezdařilo"),
    (
        "The signed-in Tomlook runtime does not offer {}; no other model was substituted",
        "Přihlášený runtime Tomlooku nenabízí {0}; jiný model nebyl použit.",
    ),
    (
        "{} does not advertise reasoning effort {}; update the isolated runtime or account entitlement",
        "{0} nenabízí úroveň přemýšlení {1}; aktualizujte izolovaný runtime nebo oprávnění účtu.",
    ),
    ("And {} more overlaps", "A další překryvy: {0}"),
    ("{}/{} available", "volno {0}/{1}"),
    (
        "Availability lookup returned HTTP {}",
        "Zjišťování dostupnosti vrátilo HTTP {0}",
    ),
    (
        "Calendar provider rejected the write with HTTP {}; nothing was changed by this request",
        "Poskytovatel odmítl zápis (HTTP {0}). Tento požadavek nic nezměnil.",
    ),
    ("AI stopped: {}", "AI byla zastavena: {0}"),
    (
        "Insights cancelled: {}. Previously saved Insights are unchanged.",
        "Zjišťování souvislostí bylo zrušeno: {0}. Dříve uložený přehled zůstává zachovaný.",
    ),
    (
        "Preparation stopped after a failed operation: {}. Inspect saved evidence and explicitly retry.",
        "Zjišťování souvislostí skončilo chybou: {0}. Zkontrolujte uložené podklady a zkuste to znovu.",
    ),
    ("HTTP {}: {}", "HTTP {0}: {1}"),
    ("MCP code {}", "Kód MCP {0}"),
    (
        "{} initialization failed: {}",
        "{0}: navázání spojení se nezdařilo: {1}",
    ),
    (
        "{} verification read failed: {}",
        "{0}: ověření čtením se nezdařilo: {1}",
    ),
    (
        "{} tool discovery failed: {}",
        "{0}: načtení nástrojů se nezdařilo: {1}",
    ),
    (
        "{} repos, {} indexed documents, {} excluded{}",
        "Repozitáře: {0}, indexované dokumenty: {1}, vynecháno: {2}{3}",
    ),
    ("Show work week of {}", "Zobrazit pracovní týden od {0}"),
    ("R{}: {}", "R{0}: {1}"),
    ("Proposed {}: {}", "Návrh – {0}: {1}"),
    (
        "One month back, six months ahead through {}.",
        "Měsíc zpět a šest měsíců dopředu, do {0}.",
    ),
    (
        "Refreshing calendar {} through {}...",
        "Načítám kalendář od {0} do {1}…",
    ),
    (
        "Preparation failed: {}. Retry {}/3 in {}s.",
        "Zjištění souvislostí se nezdařilo: {0}. Pokus {1}/3 za {2} s.",
    ),
    (
        "{} must contain 1-{} UTF-8 bytes without unsupported control characters",
        "{0} musí mít 1–{1} bajtů v UTF-8 bez nepodporovaných řídicích znaků",
    ),
    ("+{} more", "+{0} dalších"),
    ("People ({})", "Účastníci ({0})"),
    ("People ({} of {})", "Účastníci ({0} z {1})"),
    (
        "+{} companies in People",
        "Dalších {0} společností v seznamu účastníků",
    ),
    ("Icon from {}'s own website", "Ikona z webu {0}"),
    ("{} cached meetings", "Uložené schůzky: {0}"),
    ("Sources ({})", "Zdroje ({0})"),
    (
        "{} ready | {} researching | {} waiting | {} to review",
        "Hotovo {0} | Probíhá {1} | Čeká {2} | Ke kontrole {3}",
    ),
    ("{}: connected", "{0}: připojeno"),
    ("{}: disconnected", "{0}: nepřipojeno"),
    ("{}: connecting...", "{0}: připojuji…"),
    ("{}: disconnecting...", "{0}: odpojuji…"),
    (
        "{} additional tool results not retained.",
        "Dalších {0} výsledků nebylo uloženo.",
    ),
    ("Retrieved {}", "Načteno {0}"),
    ("Observed {}", "Zaznamenáno {0}"),
    (
        "{} additional results omitted by the memory limit.",
        "Dalších {0} výsledků se nevešlo do paměťového limitu.",
    ),
    ("LOCAL DRAFTS ({}/{})", "MÍSTNÍ KONCEPTY ({0}/{1})"),
    ("Local draft {}", "Místní koncept {0}"),
    ("Proposed target: {}", "Navržený cíl: {0}"),
    ("Connected as {}", "Připojeno jako {0}"),
    (
        "{}/{} cited links in captured results. Claims remain unverified.",
        "Odkazy v uložených výsledcích: {0}/{1}. Tvrzení zůstávají neověřená.",
    ),
    (
        "Locally saved calendar owned by {}; retained when disconnected.",
        "Místně uložený kalendář účtu {0}. Zůstává dostupný i bez připojení.",
    ),
    (
        "Last saved refresh: {}.",
        "Poslední uložená aktualizace: {0}.",
    ),
    ("Sync {}/{} · offline", "Načteno {0}/{1} · bez připojení"),
    ("Sync {}/{} · retrying", "Načteno {0}/{1} · opakuji"),
    ("Sync {}/{}", "Načteno {0}/{1}"),
    ("Update {} ready", "Aktualizace {0} připravena"),
    ("Last checked: {}", "Poslední kontrola: {0}"),
    ("Downloading {}: {} of {} MB", "Stahuji {0}: {1} z {2} MB"),
    ("Verifying {}...", "Ověřuji {0}…"),
    (
        "Tomlook {} is downloaded and verified.",
        "Tomlook {0} je stažený a ověřený.",
    ),
    ("Tomlook {} is available.", "K dispozici je Tomlook {0}."),
    ("GitHub returned HTTP {}.", "GitHub vrátil HTTP {0}."),
    ("In {} min · {}", "Za {0} min · {1}"),
    ("Now · {}", "Právě začíná · {0}"),
    ("{} min", "{0} min"),
    ("Next reminder: {}", "Další připomínka: {0}"),
    (
        "Last reminder shown at {}.",
        "Poslední připomínka zobrazena v {0}.",
    ),
    (
        "Test notification sent at {}.",
        "Zkušební oznámení odesláno v {0}.",
    ),
    (
        "Windows notification failed: {}",
        "Oznámení Windows se nezdařilo: {0}",
    ),
    (
        "Show the Windows notification: {}",
        "Oznámení Windows nelze zobrazit: {0}",
    ),
    (
        "Register Tomlook for Windows notifications: {}",
        "Registrace Tomlooku pro oznámení Windows se nezdařila: {0}",
    ),
    (
        "Meeting reminder history is unavailable: {}",
        "Historie připomínek není k dispozici: {0}",
    ),
    (
        "Meeting reminder history could not be saved: {}",
        "Historii připomínek nelze uložit: {0}",
    ),
    (
        "Start meeting reminders: {}",
        "Připomínky schůzek nelze spustit: {0}",
    ),
    (
        "Set the Windows notification identity: {}",
        "Identitu pro oznámení Windows nelze nastavit: {0}",
    ),
    ("{} open tasks", "Otevřené úkoly: {0}"),
    ("Recipients ({} of {})", "Příjemci ({0} z {1})"),
    ("Tasks could not be loaded: {}", "Úkoly se nepodařilo načíst: {0}"),
    (
        "Tasks from meeting notes could not be added: {}",
        "Úkoly ze zápisu ze schůzky se nepodařilo přidat: {0}",
    ),
    (
        "Could not open the e-mail app (code {})",
        "E-mailovou aplikaci se nepodařilo otevřít (kód {0})",
    ),
    (
        "Last morning overview shown: {}",
        "Poslední ranní přehled zobrazen: {0}",
    ),
    (
        "Morning overview history could not be saved: {}",
        "Historii ranního přehledu nelze uložit: {0}",
    ),
    (
        "Morning overview history is unavailable: {}",
        "Historie ranního přehledu není k dispozici: {0}",
    ),
    (
        "Compared with the average of earlier weeks: {}",
        "Srovnání s průměrem předchozích týdnů: {0}",
    ),
    ("Synced workdays: {} of 5", "Načtené pracovní dny: {0} z 5"),
    ("Overlapping time: {}", "Čas v překryvu: {0}"),
    (
        "Not checked (description arrives with the next calendar refresh): {}",
        "Neověřeno (popis přibude při příští aktualizaci kalendáře): {0}",
    ),
    ("People from this company: {}", "Lidé z této firmy: {0}"),
    ("Generated by AI: {}", "Vytvořila AI: {0}"),
    (
        "Notes of the oldest meetings left out for size: {}",
        "Kvůli velikosti vynechané zápisy nejstarších schůzek: {0}",
    ),
    (
        "Last weekly review shown: {}",
        "Poslední týdenní přehled zobrazen: {0}",
    ),
    (
        "Weekly review history could not be saved: {}",
        "Historii týdenního přehledu nelze uložit: {0}",
    ),
    (
        "Weekly review history is unavailable: {}",
        "Historie týdenního přehledu není k dispozici: {0}",
    ),
    (
        "Teams rejected the chat (HTTP {}); nothing was sent. Some people may not be reachable in Teams.",
        "Teams chat odmítl (HTTP {0}); nic nebylo odesláno. Někteří lidé možná nejsou v Teams dosažitelní.",
    ),
    (
        "Teams rejected the message (HTTP {}); the chat exists without it.",
        "Teams zprávu odmítl (HTTP {0}); chat existuje bez ní.",
    ),
    (
        "Scheduling proposals could not be loaded: {}",
        "Návrhy termínů se nepodařilo načíst: {0}",
    ),
    ("{}; nothing was deleted", "{0}; nic nebylo smazáno"),
    (
        "MSX sign-in required (Azure CLI reported {}).",
        "Je nutné přihlášení k MSX (Azure CLI hlásí {0}).",
    ),
    (
        "{} rejected the Azure CLI token; sign in to MSX again.",
        "{0} odmítlo token z Azure CLI; přihlaste se k MSX znovu.",
    ),
    ("{} is unreachable: {}", "{0} není dostupné: {1}"),
    ("{} refused the connection: {}", "{0} odmítlo připojení: {1}"),
    ("{} connection timed out", "{0}: připojení vypršelo"),
    ("{} tool discovery timed out", "{0}: načtení nástrojů vypršelo"),
    (
        "{} does not offer its read tool {} to this account.",
        "{0} tomuto účtu nenabízí nástroj pro čtení {1}.",
    ),
    ("{} read was rejected: {}", "{0}: čtení bylo odmítnuto: {1}"),
    ("Owned-task check unavailable: {}", "Kontrola vašich úkolů v MSX není k dispozici: {0}"),
    ("Owned-task check failed: {}", "Kontrola vašich úkolů v MSX selhala: {0}"),
    (
        "Prediction from the model measure [{}]; not extrapolated by Tomlook.",
        "Předpověď z míry modelu [{0}]; Tomlook ji sám neextrapoluje.",
    ),
    ("MSX: {}", "MSX: {0}"),
    ("Unconfirmed customer: {}", "Nepotvrzený zákazník: {0}"),
    ("Meetings: {}", "Schůzky: {0}"),
    ("More meetings: {}", "Další schůzky: {0}"),
    ("Keep the current MSX link: {}", "Ponechat současné propojení s MSX: {0}"),
    ("MSX search failed: {}", "Hledání v MSX se nezdařilo: {0}"),
    ("Confidence {}%", "Jistota {0} %"),
    ("Owner: {}", "Vlastník: {0}"),
    ("Website {} matches {}", "Web {0} odpovídá doméně {1}"),
    ("{} MSX contacts use @{}", "Kontakty v MSX s doménou @{1}: {0}"),
    ("Name {}% similar to {}", "Název z {0} % podobný: {1}"),
    ("Domain {} matches the name", "Doména {0} odpovídá názvu"),
    ("MSX customer: {}", "Zákazník v MSX: {0}"),
    ("Last refreshed {}", "Naposledy obnoveno {0}"),
    ("MSX unavailable: {}", "MSX není k dispozici: {0}"),
    ("Showing the saved read from {}.", "Zobrazuji uložená data z {0}."),
    ("Rows: {}, truncated", "Řádky: {0}, zkráceno"),
    ("Rows: {}", "Řádky: {0}"),
    (
        "Opportunities and milestones unavailable: {}",
        "Příležitosti a milníky nejsou k dispozici: {0}",
    ),
    ("No consumption data: {}", "Žádná data o spotřebě: {0}"),
    ("Azure consumption unavailable: {}", "Spotřeba Azure není k dispozici: {0}"),
    ("Unit: {}", "Jednotka: {0}"),
    ("Dataset: {}", "Datová sada: {0}"),
    ("Latest month with data: {}", "Poslední měsíc s daty: {0}"),
    ("Searched {} · last {} days", "Prohledáno {0} · posledních {1} dní"),
    ("Focus: {}", "Zaměření: {0}"),
    ("Why it matters: {}", "Proč je to důležité: {0}"),
    ("My commitment: {}", "Můj závazek: {0}"),
    ("Customer commitment: {}", "Závazek zákazníka: {0}"),
    (
        "{} cited sources were removed because they were not retrieved or not safe.",
        "Odstraněné citované zdroje, které nebyly načtené nebo nebyly bezpečné: {0}.",
    ),
    ("Customers could not be loaded: {}", "Zákazníky se nepodařilo načíst: {0}"),
    (
        "{} of {} required steps done · {} optional connected",
        "Povinné kroky: hotovo {0} z {1} · nepovinná připojení: {2}",
    ),
    ("Setup {}/{}", "Nastavení {0}/{1}"),
    ("Next step: {}", "Další krok: {0}"),
    (
        "Synced {} of {} days; {} not synced yet",
        "Synchronizováno {0} z {1} dní, zbývá {2}",
    ),
    ("{} of {} days", "{0} z {1} dní"),
    ("{} (version {})", "{0} (verze {1})"),
    (
        "Prerequisite check could not start: {}",
        "Kontrolu požadavků se nepodařilo spustit: {0}",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_static_controls_are_in_the_shared_catalog() {
        let sources = [
            include_str!("app/mod.rs"),
            include_str!("app/ask_panel.rs"),
            include_str!("app/calendar.rs"),
            include_str!("app/connections.rs"),
            include_str!("app/customers.rs"),
            include_str!("app/event_editor.rs"),
            include_str!("app/feedback.rs"),
            include_str!("app/find_time.rs"),
            include_str!("app/followup.rs"),
            include_str!("app/meeting.rs"),
            include_str!("app/myday.rs"),
            include_str!("app/palette.rs"),
            include_str!("app/recommendations.rs"),
            include_str!("app/settings.rs"),
            include_str!("app/setup.rs"),
            include_str!("app/tasks.rs"),
            include_str!("app/weekly.rs"),
            include_str!("ask.rs"),
        ];
        for source in sources {
            let app = source.split("#[cfg(test)]\nmod tests").next().unwrap();
            for rest in app.split("language.text(\"").skip(1) {
                let key = rest.split('"').next().unwrap();
                if !key.chars().any(|ch| ch.is_ascii_alphabetic())
                    || key.starts_with("Ctrl+")
                    || key == "Esc"
                    || key == "Tomlook"
                {
                    continue;
                }
                assert!(
                    TEXTS.iter().any(|(en, _)| *en == key),
                    "Missing Czech UI label: {key}"
                );
            }
        }
        assert_eq!(Language::Cs.text("R2: Message read"), "R2: Původní zpráva");
        assert_eq!(
            Language::Cs.text("Proposed {1}: Calendar suggestion"),
            "Návrh – {1}: Návrh do kalendáře"
        );
        assert_eq!(
            Language::Cs.text("Waiting\nWaiting for an execution slot"),
            "Čeká\nČeká na volné místo pro zpracování"
        );
        for message in crate::updater::MESSAGES {
            assert!(
                TEXTS.iter().any(|(en, _)| en == message),
                "Missing Czech update message: {message}"
            );
        }
        assert_eq!(
            Language::Cs.text("Downloading 0.8.0: 1.5 of 20.0 MB"),
            "Stahuji 0.8.0: 1.5 z 20.0 MB"
        );
        assert_eq!(
            Language::Cs.text("Update 0.8.0 ready"),
            "Aktualizace 0.8.0 připravena"
        );
    }

    #[test]
    fn appearance_language_and_ghost_preference_survive_restart() {
        let root = tempfile::tempdir().unwrap();
        let preferences = crate::storage::Preferences {
            language: Language::Cs,
            hide_unconfirmed: true,
            dark: false,
            accent: 2,
            ..Default::default()
        };
        crate::storage::Store::open(root.path())
            .unwrap()
            .save_preferences(&preferences)
            .unwrap();
        let restored = crate::storage::Store::open(root.path())
            .unwrap()
            .preferences()
            .unwrap();
        assert_eq!(restored.language, Language::Cs);
        assert!(restored.hide_unconfirmed);
        assert!(!restored.dark);
        assert_eq!(restored.accent, 2);
    }

    #[test]
    fn trailing_placeholders_keep_values_that_contain_the_closing_text() {
        assert_eq!(
            Language::Cs.text("Last reminder shown at 31.10.2026 21:39."),
            "Poslední připomínka zobrazena v 31.10.2026 21:39."
        );
        assert_eq!(Language::Cs.text("In 5 min · 09:15–10:45"), "Za 5 min · 09:15–10:45");
    }

    #[test]
    fn recovery_connector_and_source_statuses_are_localized_without_changing_codes() {
        assert_eq!(
            Language::Cs.text("Enter ABCD-EFGH at github.com/login/device"),
            "Zadejte kód ABCD-EFGH na github.com/login/device"
        );
        assert_eq!(
            Language::Cs.text("AI sign-in failed: provider unavailable"),
            "Přihlášení k AI selhalo: provider unavailable"
        );
        assert_eq!(Language::Cs.text("AI sign-in completed."), "Přihlášení k AI proběhlo úspěšně.");
        assert_eq!(
            Language::Cs
                .text("WorkIQ connection interrupted.\nAutomatic reconnect in about 60 seconds."),
            "Připojení WorkIQ bylo přerušeno.\nDalší automatický pokus přibližně za 60 s."
        );
        assert_eq!(
            Language::Cs.text("Connector has no supported verification read"),
            "Konektor nenabízí podporovaný nástroj pro ověření přístupu."
        );
        assert_eq!(
            Language::Cs.text(
                "GitHub MCP did not expose the account verification tool (get_me). Check the context toolset; this is not a rejected sign-in."
            ),
            "GitHub MCP nenabízí nástroj pro ověření účtu (get_me). Zkontrolujte sadu nástrojů context; nejde o odmítnuté přihlášení."
        );
        assert_eq!(
            Language::Cs.text(
                "Interrupted provider operation; inspect saved evidence and explicitly retry before resuming preparation"
            ),
            "Zjišťování bylo přerušeno. Prohlédněte uložené podklady a teprve potom je výslovně spusťte znovu."
        );
        assert_eq!(
            Language::Cs
                .text("WebIQ verification read failed: HTTP 429: provider rate limit; retry later"),
            "WebIQ: ověření čtením se nezdařilo: HTTP 429: poskytovatel omezil počet požadavků; zkuste to později"
        );
        for (partial, errors, suffix) in [
            (false, false, ""),
            (true, false, " (neúplné)"),
            (false, true, " – chyby zdrojů"),
            (true, true, " (neúplné) – chyby zdrojů"),
        ] {
            let report = crate::source_index::Report {
                repositories: 134,
                documents: 32985,
                skipped: 7,
                truncated: partial,
                errors: if errors {
                    vec!["Original provider detail".into()]
                } else {
                    vec![]
                },
                ..Default::default()
            };
            assert_eq!(
                Language::Cs.text(&report.label()),
                format!("Repozitáře: 134, indexované dokumenty: 32985, vynecháno: 7{suffix}")
            );
        }
        assert_eq!(
            Language::Cs.text("Original provider detail: request-id abc"),
            "Original provider detail: request-id abc"
        );
    }

    #[test]
    fn catalogs_are_unique_nonempty_and_templates_preserve_arguments() {
        let mut seen = std::collections::BTreeSet::new();
        for &(en, cs) in TEXTS {
            assert!(seen.insert(en), "Duplicate English key: {en}");
            assert!(!cs.trim().is_empty());
            assert_eq!(Language::En.text(en), en);
            assert_eq!(Language::Cs.text(en), cs);
        }
        for &(en, cs) in PATTERNS {
            let count = en.matches("{}").count();
            for index in 0..count {
                assert!(cs.contains(&format!("{{{index}}}")));
            }
        }
        assert_eq!(
            Language::Cs.text("13 ready | 1 researching | 394 waiting | 1 to review"),
            "Hotovo 13 | Probíhá 1 | Čeká 394 | Ke kontrole 1"
        );
        assert_eq!(
            Language::Cs.text("A customer-provided title"),
            "A customer-provided title"
        );
        assert_eq!(
            Language::Cs.text("Insights cancelled: Cancelled before the question was sent. Previously saved Insights are unchanged."),
            "Zjišťování souvislostí bylo zrušeno: Zrušeno před odesláním dotazu. Dříve uložený přehled zůstává zachovaný."
        );
    }

    #[test]
    fn msx_statuses_and_provider_failures_are_localized_without_changing_codes() {
        use crate::msx::{Phase, Status, auth};
        for phase in [
            Phase::NotConnected,
            Phase::Checking,
            Phase::SigningIn,
            Phase::Connected,
            Phase::SignInRequired,
            Phase::Unavailable,
            Phase::Disabled,
        ] {
            let status = Status {
                phase,
                ..Default::default()
            };
            assert_ne!(Language::Cs.text(status.label()), status.label(), "{phase:?}");
            assert_ne!(Language::Cs.text(phase.label()), phase.label(), "{phase:?}");
        }
        for message in [
            crate::msx::NOT_CONNECTED,
            crate::msx::CHECKING,
            crate::msx::CONNECTED,
            crate::msx::DATAVERSE_ONLY,
            crate::msx::POWERBI_ONLY,
            crate::msx::DISABLED,
            crate::msx::DEMO,
            crate::msx::DISCONNECTED,
            crate::msx::BROWSER_PROMPT,
            crate::msx::DEVICE_REQUEST,
            crate::msx::DEVICE_PROMPT,
            crate::msx::CHECK_CANCELLED,
            crate::msx::INVALID_TENANT,
            auth::SIGN_IN_REQUIRED,
            auth::TOKEN_TIMEOUT,
            auth::CLI_MISSING,
            auth::CLI_FAILED,
            auth::TENANT_MISMATCH,
            auth::INVALID_TOKEN,
            auth::NETWORK,
            auth::LOGIN_FAILED,
            auth::LOGIN_CANCELLED,
            auth::LOGIN_TIMED_OUT,
            auth::DEVICE_CODE_TIMEOUT,
            "Connected (read-only).",
            "Dataverse read timed out",
            "Power BI read timed out",
            "Reading MSX (Dataverse)",
            "Reading Azure consumption (Power BI)",
        ] {
            assert!(
                TEXTS.iter().any(|(en, _)| *en == message),
                "Missing Czech MSX message: {message}"
            );
        }
        assert_eq!(
            Language::Cs.text("MSX sign-in required (Azure CLI reported AADSTS50076)."),
            "Je nutné přihlášení k MSX (Azure CLI hlásí AADSTS50076)."
        );
        assert_eq!(
            Language::Cs.text("Dataverse is unreachable: HTTP 503: provider unavailable; retry later"),
            format!(
                "Dataverse není dostupné: {}",
                Language::Cs.text("HTTP 503: provider unavailable; retry later")
            )
        );
        assert_eq!(
            Language::Cs.text("Power BI rejected the Azure CLI token; sign in to MSX again."),
            "Power BI odmítlo token z Azure CLI; přihlaste se k MSX znovu."
        );
        assert_eq!(
            Language::Cs.text(
                "Dataverse does not offer its read tool read_query to this account."
            ),
            "Dataverse tomuto účtu nenabízí nástroj pro čtení read_query."
        );
        assert_eq!(
            Language::Cs.text("WorkIQ context read timed out"),
            "WorkIQ context read timed out",
            "MSX wording does not capture other providers' messages"
        );
    }

    #[test]
    fn legacy_preferences_default_to_english_and_ghosts_remain_visible() {
        let preferences: crate::storage::Preferences =
            serde_json::from_str(r#"{"dark":false}"#).unwrap();
        assert_eq!(preferences.language, Language::En);
        assert!(!preferences.hide_unconfirmed);
        assert!(preferences.automatic_update_check);
        assert!(preferences.update_checked_at.is_none());
        let date = NaiveDate::from_ymd_opt(2026, 10, 11).unwrap();
        assert_eq!(Language::Cs.month(date), "říjen 2026");
        assert_eq!(Language::Cs.date(date, true), "neděle, 11. 10. 2026");
    }
}
