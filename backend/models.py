from __future__ import annotations

from datetime import datetime
from enum import StrEnum
from typing import Literal
from urllib.parse import urlsplit

from pydantic import BaseModel, Field, field_validator


def safe_source_url(value: object) -> str | None:
    if not isinstance(value, str) or any(ord(char) < 32 for char in value):
        return None
    try:
        parsed = urlsplit(value)
        if parsed.scheme in {"http", "https"} and parsed.hostname and not parsed.username and not parsed.password:
            return value
    except ValueError:
        pass
    return None


class EvidenceType(StrEnum):
    FACT = "fact"
    SYNTHESIS = "synthesis"
    INFERENCE = "inference"
    RECOMMENDATION = "recommendation"
    MISSING = "missing"


class SourceReference(BaseModel):
    id: str
    connector: str
    title: str
    timestamp: datetime
    evidence_type: EvidenceType
    url: str | None = None
    is_mock: bool = False

    @field_validator("url", mode="before")
    @classmethod
    def validate_url(cls, value: object) -> str | None:
        return safe_source_url(value)


class CalendarEvent(BaseModel):
    id: str
    title: str
    start: datetime
    end: datetime
    category: str
    attendees: list[str]
    organizer: str
    location: str
    status: str
    briefing_status: str
    is_all_day: bool = False
    has_changes: bool = False
    source_url: str | None = None
    metadata_warnings: list[str] = Field(default_factory=list)

    @field_validator("source_url", mode="before")
    @classmethod
    def validate_source_url(cls, value: object) -> str | None:
        return safe_source_url(value)


class RoleEstimate(BaseModel):
    role: str
    confidence: float = Field(ge=0, le=1)
    explanation: str
    evidence_source_ids: list[str]


class ConsumptionPoint(BaseModel):
    label: str
    value: float = Field(allow_inf_nan=False)
    kind: Literal["actual", "partial_actual", "prediction", "unspecified"] = "unspecified"


class TaskDraft(BaseModel):
    subject: str
    due_date: str
    duration_hours: float = Field(ge=0)
    activity_type: str
    milestone_id: str
    reason: str
    status: str = "draft"


class MilestoneContext(BaseModel):
    id: str
    name: str
    due_date: str
    status: str
    opportunity: str | None = None
    confidence: float = Field(ge=0, le=1)
    association_reason: str
    has_user_task: bool | None = None
    existing_task_subject: str | None = None
    suggested_task: TaskDraft | None = None
    source_ids: list[str] = Field(default_factory=list)
    url: str | None = None

    @field_validator("url", mode="before")
    @classmethod
    def validate_url(cls, value: object) -> str | None:
        return safe_source_url(value)


class Briefing(BaseModel):
    meeting_id: str
    version: int
    created_at: datetime
    status: str
    summary: str
    objective: str
    why_now: str
    role: RoleEstimate
    preparation: list[str]
    open_questions: list[str]
    risks: list[str]
    decisions: list[str]
    talking_points: list[str]
    communication_context: list[str]
    business_context: dict[str, str]
    consumption: list[ConsumptionPoint]
    consumption_unit: str
    consumption_period: str
    public_context: list[str]
    changes: list[str]
    sources: list[SourceReference]
    warnings: list[str]
    claim_sources: dict[str, list[str]] = Field(default_factory=dict)
    claim_confidence: dict[str, float] = Field(default_factory=dict)
    milestones: list[MilestoneContext] = Field(default_factory=list)
    analysis_profile: str = "base-v1"
    input_fingerprint: str | None = None


class MeetingDetail(BaseModel):
    event: CalendarEvent
    invitation: str
    response_status: str
    briefing: Briefing


class ChatTurn(BaseModel):
    role: Literal["user", "assistant"]
    text: str = Field(min_length=1, max_length=4000)


class ChatRequest(BaseModel):
    message: str = Field(min_length=1, max_length=2000)
    history: list[ChatTurn] = Field(default_factory=list, max_length=12)


class ChatResponse(BaseModel):
    id: str
    answer: str
    sources: list[SourceReference]
    can_add_to_briefing: bool = True


class FeedbackRequest(BaseModel):
    message: str = Field(min_length=1, max_length=2000)
    scope: str = Field(pattern="^(meeting|preference|skill)$")


class SkillProposal(BaseModel):
    id: str
    title: str
    reason: str
    status: str
    old_content: str
    new_content: str
    created_at: datetime


class ConnectorCapability(BaseModel):
    id: str
    label: str
    capabilities: list[str]
    status: str
    mode: str = "live"
    last_checked: datetime
    detail: str | None = None


class HealthResponse(BaseModel):
    status: str
    runtime: str
    database: str
    mode: str
    active_jobs: int


class SetupStatus(BaseModel):
    ready: bool
    missing: list[str]
    configured_connectors: list[str]
    instructions: list[str]
