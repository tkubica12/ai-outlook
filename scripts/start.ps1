$ErrorActionPreference = "Stop"

if (-not (Test-Path ".venv")) {
    python -m venv .venv
}

& .\.venv\Scripts\python.exe -m pip install -e ".[dev]" --quiet
if (-not (Test-Path "node_modules")) {
    npm install
}

$backend = Start-Process -FilePath ".\.venv\Scripts\python.exe" `
    -ArgumentList "-m", "uvicorn", "backend.app:app", "--reload", "--host", "127.0.0.1", "--port", "8000" `
    -PassThru

try {
    npm run dev
}
finally {
    if (-not $backend.HasExited) {
        Stop-Process -Id $backend.Id
    }
}

