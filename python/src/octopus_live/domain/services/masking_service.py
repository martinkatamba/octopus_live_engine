import copy
from typing import Any, Dict, List, Optional, Set


def mask_payload(
    payload: Optional[Dict[str, Any]],
    mask_fields: List[str],
    mask_value: str = "********"
) -> Optional[Dict[str, Any]]:
    """
    Creates a deep copy of payload and replaces values for any keys matching
    mask_fields (case-insensitive) with mask_value (defaulting to '********').
    """
    if payload is None:
        return None
    if not mask_fields:
        return copy.deepcopy(payload)
    if not mask_value:
        mask_value = "********"

    target_keys: Set[str] = {
        str(f).strip().lower()
        for f in mask_fields
        if str(f).strip()
    }

    return _sanitize_dict(payload, target_keys, mask_value)


def _sanitize_dict(
    d: Dict[str, Any],
    targets: Set[str],
    mask_value: str
) -> Dict[str, Any]:
    result: Dict[str, Any] = {}
    for k, v in d.items():
        k_str = str(k).strip().lower()
        if k_str in targets:
            result[k] = mask_value
        else:
            result[k] = _sanitize_value(v, targets, mask_value)
    return result


def _sanitize_value(
    v: Any,
    targets: Set[str],
    mask_value: str
) -> Any:
    if v is None:
        return None
    if isinstance(v, dict):
        return _sanitize_dict(v, targets, mask_value)
    if isinstance(v, list):
        return [_sanitize_value(item, targets, mask_value) for item in v]
    if isinstance(v, tuple):
        return tuple(_sanitize_value(item, targets, mask_value) for item in v)
    return v
