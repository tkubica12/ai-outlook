# Připojení GitHubu k Tomlooku

Tomlook potřebuje vlastní identitu klienta pro přihlášení přes prohlížeč.
Copilot ji má registrovanou od GitHubu; Tomlook ji ani vaše uložené přihlášení
nepřebírá. Dialog obsahuje jen stav připojení, zapamatované **Client ID**
a tlačítko **Přihlásit se**. Přepínání způsobu přihlášení ani registrační
tlačítka v něm nejsou.

## Registrace pro přihlášení v prohlížeči

1. Otevřete [New GitHub App](https://github.com/settings/apps/new)
   ve svém účtu: **Settings → Developer settings → GitHub Apps**.
   Název musí být jedinečný, například `Tomlook-vas-login`.
2. Do **Homepage URL** vložte adresu veřejného repozitáře aplikace nebo
   profilu vlastníka. **Callback URL** a **Setup URL** můžete ponechat prázdné:
   Tomlook používá device flow, nikoli přesměrování na vlastní webový server.
3. Zapněte **Enable Device Flow**. **Expire user authorization tokens**
   ponechte zapnuté. **Request user authorization (OAuth) during installation**
   není pro tento postup potřeba.
4. V části **Webhook** vypněte **Active**. Tomlook nepotřebuje webhook URL
   ani webhook secret.
5. V **Repository permissions** nastavte **Contents**, **Issues** a
   **Pull requests** na **Read-only**. **Metadata** je povinné čtení GitHubu.
   Ostatní oprávnění ponechte na **No access**; nepovolujte zápis.
6. Pro repozitáře jen ve vlastním účtu zvolte **Only on this account**.
   Potřebujete-li také repozitáře jiné organizace, je nutné **Any account**
   a instalace schválená vlastníkem dané organizace. Tato volba sama o sobě
   přístup k žádnému repozitáři nedává.
7. Klikněte **Create GitHub App**. Potom v **Install App** nainstalujte
   aplikaci s volbou **Only select repositories** jen do potřebných repozitářů.
   Registrace, instalace a uživatelské přihlášení jsou tři odlišné kroky.
8. Na stránce aplikace zkopírujte **Client ID**, nikoli číselné **App ID**.
   Nevytvářejte pro Tomlook client secret ani private key; tento device flow
   je nepotřebuje.
9. V Tomlooku klikněte dole na text **GitHub**, ověřte předvyplněné
   Client ID (při prvním nastavení ho vložte) a klikněte **Přihlásit se**.
   V prohlížeči zadejte
   zobrazený kód, ověřte správný účet a schvalte přístup.

Platné Client ID se pamatuje odděleně od přihlašovacích tokenů, také po
neúspěšném nebo zrušeném přihlášení. Tomlook ho umí převzít i ze starého
uloženého přihlášení, jehož token už vypršel. Během autorizace se tlačítko
změní na **Zrušit** a stav ukáže potřebný kód. Po připojení je přihlášení
neaktivní. Podrobnosti chyb najdete pod trojúhelníkem ve **Zprávách**.

Tomlook před uložením ověřuje shodnou identitu přes GitHub API i read-only MCP.
Úspěšné připojení neprokazuje dostupnost každého repozitáře.
Při organizačním SSO může být nutné nejdřív aktivovat SAML relaci nebo získat
schválení administrátora. Klient ani souhlas za vás aplikace nevytváří.

Výchozí uživatelský token GitHub App platí osm hodin. Tomlook ukládá access token
i refresh token šifrovaně pomocí Windows DPAPI pro váš účet, odděleně od Copilotu
a `gh`. Přihlášení obnovuje na pozadí přibližně pět minut před vypršením,
také při spuštění po delší přestávce. Device flow nepotřebuje client secret
ani při obnově. Obnova tokenu stejného účtu sama o sobě nezneplatňuje přehledy.
Kvůli pohodlí nevypínejte expiraci.

**Přechod ze starší verze:** ta refresh token neukládala. Přihlaste se proto
jednou znovu stejným Client ID. Potom už Tomlook provádí běžnou obnovu sám.
Znovu se přihlásit bude nutné po odvolání přístupu nebo po skončení platnosti
refresh tokenu (výchozí platnost šest měsíců). Osobní token se tímto postupem
neobnovuje; jeho expiraci spravujete v GitHubu.

## Web hlásí úspěch, ale Tomlook připojení nedokončí

Úspěch na stránce GitHubu potvrzuje autorizaci. Tomlook potom ještě ověřuje
shodný účet přes GitHub API a MCP; teprve po úspěchu uloží přihlášení.
Chyba **Connector has no supported verification read** ve starší verzi
Tomlooku vznikala vynecháním sady MCP nástrojů `context`, která obsahuje `get_me`.
Aktuální verze ji požaduje při ověření i v AI sessions. GitHub App není potřeba
vytvářet znovu ani rozšiřovat její oprávnění; spusťte aktuální
`release\Tomlook.exe` a zopakujte přihlášení se stejným Client ID.
Neúspěšný ověřovací pokus nový token neuložil.

## Dříve uložené osobní tokeny

Dříve uložené osobní tokeny se dál obnovují z šifrovaného úložiště a ověřují
pouze pro čtení. Zjednodušený dialog už jejich zadávání nenabízí. Nové
přihlášení používá GitHub App a device flow. Token neposílejte do chatu ani
do repozitáře. Přístup můžete odvolat v nastavení GitHubu; zjednodušený dialog
nemá další tlačítko pro odstranění přihlášení.

## Podklady a omezení

Postup vychází z veřejné dokumentace GitHubu a implementace Tomlooku.
Neověřuje vaše organizační zásady ani skutečnou instalaci. Pracovní a komunitní
zdroje nejsou pro tento registrační postup potřeba a nebyly pro něj prohledávány.

| Veřejný zdroj | Zjištění | Získání |
| --- | --- | --- |
| [Registering a GitHub App](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/registering-a-github-app) | Device Flow, nepovinný callback, vypnutí webhooku, minimální oprávnění, rozsah instalace | WebIQ browse, 1 stránka; datum vydání neuvedeno |
| [Installing your own GitHub App](https://docs.github.com/en/apps/using-github-apps/installing-your-own-github-app) | Install App, omezení na vybrané repozitáře a význam viditelnosti aplikace | WebIQ browse, 1 stránka; datum vydání neuvedeno |
| [Generating a user access token](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app) | Client ID není App ID, průnik přístupů uživatele a aplikace, výchozí osmihodinová platnost a SAML | WebIQ browse, 1 stránka; načtený úsek, nikoli celý dokument |
| [Authorizing OAuth apps – Device flow](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps#device-flow) | Device flow používá Client ID bez client secret | WebIQ web, 2 výsledky stejné dokumentace; relevantní pasáž |
| [GitHub MCP Server – Context](https://github.com/github/github-mcp-server/blob/main/README.md#tools) | `get_me` patří do sady `context` a nepotřebuje parametry | `gh api`, 1 veřejný soubor; relevantní pasáž, datum vydání neuvedeno |
| [GitHub Remote MCP Server – Headers](https://github.com/github/github-mcp-server/blob/main/docs/remote-server.md#headers) | `X-MCP-Toolsets` vybírá kombinaci sad nástrojů, `X-MCP-Readonly` zachovává režim čtení | `gh api`, 1 veřejný soubor; relevantní pasáž, datum vydání neuvedeno |
| [Refreshing user access tokens](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/refreshing-user-access-tokens) | Osm hodin pro access token, šest měsíců pro refresh token, rotace obou tokenů; device flow nepotřebuje client secret | `web_fetch`, 1 veřejná stránka, 12. 10. 2026; datum vydání neuvedeno |
