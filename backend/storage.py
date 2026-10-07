from __future__ import annotations

import json
import os
import sqlite3
from pathlib import Path

from backend.models import Briefing, FeedbackRequest, SkillProposal

class Storage:
    def __init__(self, path: Path | None = None):
        path = path or Path(os.getenv("OUTLOOK_NEXT_DB", "data/outlook-next.db"))
        self.path = path
        path.parent.mkdir(parents=True, exist_ok=True)
        self.connection = sqlite3.connect(path, check_same_thread=False)
        self.connection.row_factory = sqlite3.Row
        self.connection.executescript(
            """
            CREATE TABLE IF NOT EXISTS briefings (
              meeting_id TEXT NOT NULL,
              version INTEGER NOT NULL,
              payload TEXT NOT NULL,
              created_at TEXT NOT NULL,
              PRIMARY KEY (meeting_id, version)
            );
            CREATE TABLE IF NOT EXISTS feedback (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              meeting_id TEXT NOT NULL,
              message TEXT NOT NULL,
              scope TEXT NOT NULL,
              created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS skill_proposals (
              id TEXT PRIMARY KEY,
              payload TEXT NOT NULL,
              status TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS analysis_jobs (
              id TEXT PRIMARY KEY,
              meeting_id TEXT NOT NULL,
              payload TEXT NOT NULL,
              created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS analysis_jobs_meeting
              ON analysis_jobs(meeting_id, created_at DESC);
            """
        )
        self.connection.commit()

    def save_briefing(self, item: Briefing) -> None:
        with self.connection:
            self.connection.execute(
                "INSERT INTO briefings (meeting_id, version, payload, created_at) "
                "VALUES (?, ?, ?, ?)",
                (
                    item.meeting_id,
                    item.version,
                    item.model_dump_json(),
                    item.created_at.isoformat(),
                ),
            )

    def save_job(self, job: dict[str, str]) -> None:
        self.connection.execute(
            "INSERT OR REPLACE INTO analysis_jobs VALUES (?, ?, ?, ?)",
            (job["id"], job["meeting_id"], json.dumps(job), job["created_at"]),
        )
        self.connection.commit()

    def get_job(self, job_id: str) -> dict[str, str] | None:
        row = self.connection.execute(
            "SELECT payload FROM analysis_jobs WHERE id = ?", (job_id,)
        ).fetchone()
        return json.loads(row["payload"]) if row else None

    def latest_job(self, meeting_id: str) -> dict[str, str] | None:
        row = self.connection.execute(
            "SELECT payload FROM analysis_jobs WHERE meeting_id = ? ORDER BY created_at DESC LIMIT 1",
            (meeting_id,),
        ).fetchone()
        return json.loads(row["payload"]) if row else None

    def recover_jobs(self) -> None:
        rows = self.connection.execute(
            "SELECT payload FROM analysis_jobs WHERE json_extract(payload, '$.status') IN ('queued', 'running')"
        ).fetchall()
        for row in rows:
            job = json.loads(row["payload"])
            job.update(status="failed", detail="Analysis was interrupted by a backend restart. Retry when ready.")
            self.save_job(job)

    def reset(self) -> None:
        """Drops all stored demonstration state. Used only by the test harness."""
        self.connection.executescript(
            "DELETE FROM briefings; DELETE FROM feedback; DELETE FROM skill_proposals;"
        )
        self.connection.commit()

    def latest_briefing(self, meeting_id: str) -> Briefing | None:
        row = self.connection.execute(
            "SELECT payload FROM briefings WHERE meeting_id = ? ORDER BY version DESC LIMIT 1",
            (meeting_id,),
        ).fetchone()
        return Briefing.model_validate_json(row["payload"]) if row else None

    def history(self, meeting_id: str) -> list[Briefing]:
        rows = self.connection.execute(
            "SELECT payload FROM briefings WHERE meeting_id = ? ORDER BY version DESC",
            (meeting_id,),
        ).fetchall()
        return [Briefing.model_validate_json(row["payload"]) for row in rows]

    def save_feedback(self, meeting_id: str, item: FeedbackRequest) -> None:
        self.connection.execute(
            "INSERT INTO feedback (meeting_id, message, scope) VALUES (?, ?, ?)",
            (meeting_id, item.message, item.scope),
        )
        self.connection.commit()

    def save_proposal(self, proposal: SkillProposal) -> None:
        self.connection.execute(
            "INSERT OR REPLACE INTO skill_proposals (id, payload, status) VALUES (?, ?, ?)",
            (proposal.id, proposal.model_dump_json(), proposal.status),
        )
        self.connection.commit()

    def get_proposal(self, proposal_id: str) -> SkillProposal | None:
        row = self.connection.execute(
            "SELECT payload, status FROM skill_proposals WHERE id = ?", (proposal_id,)
        ).fetchone()
        if not row:
            return None
        data = json.loads(row["payload"])
        data["status"] = row["status"]
        return SkillProposal.model_validate(data)

    def update_proposal(self, proposal_id: str, status: str) -> SkillProposal | None:
        self.connection.execute(
            "UPDATE skill_proposals SET status = ? WHERE id = ?", (status, proposal_id)
        )
        self.connection.commit()
        return self.get_proposal(proposal_id)
