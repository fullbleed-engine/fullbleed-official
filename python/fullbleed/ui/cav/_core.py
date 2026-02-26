# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Fullbleed-Commercial
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Iterable, Mapping, Protocol


@dataclass(frozen=True)
class CavProfile:
    """County/revision-scoped CAV profile contract.

    Claims of support/completeness attach to the profile, not the family kit.
    """

    profile_id: str
    profile_version: int
    family_id: str
    revision: str
    jurisdiction: str
    county: str | None = None
    issuing_authority: str | None = None
    display_name: str | None = None
    supported_variants: tuple[str, ...] = ()
    unsupported_features: tuple[str, ...] = ()
    coverage_notes: tuple[str, ...] = ()
    strict_scope_default: bool = True

    def as_metadata(self) -> dict[str, Any]:
        return {
            "profile_id": self.profile_id,
            "profile_version": self.profile_version,
            "family_id": self.family_id,
            "revision": self.revision,
            "jurisdiction": self.jurisdiction,
            "county": self.county,
            "issuing_authority": self.issuing_authority,
            "display_name": self.display_name or self.profile_id,
            "supported_variants": list(self.supported_variants),
            "unsupported_features": list(self.unsupported_features),
            "coverage_notes": list(self.coverage_notes),
            "strict_scope_default": self.strict_scope_default,
        }


class RenderableArtifact(Protocol):
    def to_html(self, *args: Any, **kwargs: Any) -> str: ...


def _normalize_mapping_keys(payload: Mapping[str, Any] | None) -> set[str]:
    if not isinstance(payload, Mapping):
        return set()
    return {str(k) for k in payload.keys()}


def _coerce_str_list(values: Iterable[str] | None) -> tuple[str, ...]:
    if not values:
        return ()
    return tuple(str(v) for v in values)


@dataclass
class CavKitBase:
    """Base contract for document-family CAV kits.

    Public chunk size is the family kit. Smaller sections should usually remain
    internal partials in the family module/package.
    """

    profile: CavProfile
    strict_scope: bool | None = None
    allowed_payload_fields: tuple[str, ...] = field(default_factory=tuple)

    family_id: str = field(init=False, default="")

    def __post_init__(self) -> None:
        if not self.family_id:
            raise TypeError(f"{type(self).__name__} must define class field 'family_id'")
        if self.profile.family_id != self.family_id:
            raise ValueError(
                f"profile family mismatch: expected '{self.family_id}', got '{self.profile.family_id}'"
            )
        if self.strict_scope is None:
            self.strict_scope = bool(self.profile.strict_scope_default)

    def profile_metadata(self) -> dict[str, Any]:
        return self.profile.as_metadata()

    def validate_payload_scope(self, payload: Mapping[str, Any] | None) -> dict[str, Any]:
        allowed = set(self.allowed_payload_fields)
        observed = _normalize_mapping_keys(payload)
        extra = sorted(observed - allowed) if allowed else []
        issues: list[dict[str, Any]] = []
        if extra:
            issues.append(
                {
                    "code": "CAVKIT_PROFILE_UNMAPPED_PAYLOAD_FIELD",
                    "severity": "error" if self.strict_scope else "warn",
                    "fields": extra,
                    "profile_id": self.profile.profile_id,
                    "family_id": self.family_id,
                }
            )
        return {
            "ok": not any(i["severity"] == "error" for i in issues),
            "issues": issues,
            "profile": self.profile_metadata(),
        }

    def render(
        self,
        *,
        payload: Mapping[str, Any],
        claim_evidence: Mapping[str, Any] | None = None,
    ) -> RenderableArtifact:
        raise NotImplementedError


@dataclass(frozen=True)
class CavProfileRegistry:
    profiles: tuple[CavProfile, ...]

    def by_id(self, profile_id: str) -> CavProfile:
        for profile in self.profiles:
            if profile.profile_id == profile_id:
                return profile
        raise KeyError(profile_id)

    def list_ids(self, *, family_id: str | None = None) -> list[str]:
        ids = []
        for profile in self.profiles:
            if family_id and profile.family_id != family_id:
                continue
            ids.append(profile.profile_id)
        return ids


def profile_registry(*profiles: CavProfile) -> CavProfileRegistry:
    return CavProfileRegistry(tuple(profiles))

