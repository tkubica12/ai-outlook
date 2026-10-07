from __future__ import annotations

from abc import ABC, abstractmethod

from backend.models import Briefing, ChatResponse


class AgentRuntime(ABC):
    @abstractmethod
    async def refresh(self, current: Briefing) -> Briefing:
        raise NotImplementedError

    @abstractmethod
    async def chat(self, briefing: Briefing, message: str) -> ChatResponse:
        raise NotImplementedError


class UnconfiguredAgentRuntime(AgentRuntime):
    async def refresh(self, current: Briefing) -> Briefing:
        raise RuntimeError("AI agent runtime is not configured")

    async def chat(self, briefing: Briefing, message: str) -> ChatResponse:
        raise RuntimeError("AI agent runtime is not configured")
