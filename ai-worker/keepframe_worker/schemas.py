from __future__ import annotations

from typing import Literal

from pydantic import BaseModel, Field


class AnalysisResponse(BaseModel):
    observations: list[str] = Field(min_length=1, max_length=8)
    suggested_intents: list[
        Literal[
            "restoration", "scratch_repair", "denoise", "sharpen", "upscale",
            "lighting_correction", "object_removal", "sky_replacement",
            "colourisation", "custom",
        ]
    ] = Field(min_length=1, max_length=4)
    preserve: list[
        Literal[
            "identity_faces", "composition", "text", "period_detail",
            "skin_texture", "grain", "monochrome_tonality",
        ]
    ] = Field(min_length=1, max_length=7)
    negative_constraints: list[str] = Field(min_length=1, max_length=8)


def normalise_payload(payload: dict, requested_intent: str) -> AnalysisResponse:
    payload = dict(payload)
    payload.setdefault("suggested_intents", [requested_intent])
    payload.setdefault("preserve", ["identity_faces", "composition"])
    payload.setdefault("negative_constraints", [
        "Do not reshape faces or change identity.",
        "Do not invent objects, text, jewellery or historical details.",
    ])
    return AnalysisResponse.model_validate(payload)
