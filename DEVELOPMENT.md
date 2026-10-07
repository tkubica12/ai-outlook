# Development

The frontend is React 19, TypeScript, and Vite. The backend is Python 3.12, FastAPI,
Pydantic, and SQLite. Run both with `scripts/start.ps1`; Vite proxies `/api` to FastAPI.

Use `npm run build` for TypeScript checking and production assets, `npm run lint` for
static analysis, `npm test` for unit and component tests, and `pytest` for API/model
behavior. Playwright covers connection onboarding, calendar navigation, event reachability,
and desktop/mobile interactions.

Playwright uses `OUTLOOK_NEXT_DB=data/e2e.db` and requires no external credentials. If a backend
is already listening on port 8000, Playwright reuses it. For isolated disconnected tests stop the
local servers first; route-mocked UI scenarios do not write to external services.

Repository layout:

```text
backend/       FastAPI, models, storage, connectors, runtime, tests
src/           React app, components, date logic, API client, styles, unit tests
e2e/           Playwright live-connection onboarding
.github/skills/ business context procedures included in briefing prompts
skills/        additional human-readable meeting procedure
scripts/       local startup automation
```
