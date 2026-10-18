"""Typed wanda.decision_v1 strategic analysis parser, host envelope, and Cortex projection.

Implements _dev/verification/rtx3070/2026-10-16-wanda-cortex-decision-design.md:
- Core's 6-field advisory gate and Wanda's existing Recommendation/QueryResponse remain
  unchanged.
- Validates model-generated strategic analysis against admitted domain evidence refs.
- Host-authored envelope enforces schema_version=1, kind="wanda.decision_v1", 16 KiB
  UTF-8 ceiling, bounded identifiers, 24h max horizon shortened to admitted evidence
  expiry, and strict structural checks (no moral filter).
- Produces companion trusted correlation records and inferred Document node projections
  for Cortex without granting independent evidence votes.
"""
import json
import re
from .contracts import AdmissionError, digest

SCHEMA_VERSION = 1
KIND = 'wanda.decision_v1'
MAX_ENVELOPE_BYTES = 16 * 1024
MAX_HORIZON_MS = 24 * 3_600_000
ID_RE = re.compile(r'^[A-Za-z0-9._:/-]{1,192}$')

ENVELOPE_KEYS = {
    'schema_version', 'kind', 'decision_id', 'task_id', 'domain',
    'produced_at_ms', 'expires_at_ms', 'source_run_ref',
    'objective_refs', 'evidence_refs', 'analysis',
}
ANALYSIS_KEYS = {
    'user_objectives', 'alternatives', 'selected_option_id',
    'justification', 'uncertainty', 'revision_conditions',
}
OBJECTIVE_KEYS = {'objective_ref', 'priority', 'horizon', 'summary'}
ALTERNATIVE_KEYS = {
    'option_id', 'action_summary', 'supporting_refs',
    'uncertainties', 'consequences', 'relational_costs',
}
OUTCOME_KEYS = {'description', 'horizon', 'evidence_refs'}
UNCERTAINTY_KEYS = {'missing_information', 'competing_explanations', 'forecasts'}
FORECAST_KEYS = {'event', 'probability_permille', 'resolution_condition', 'evidence_refs'}


def _bounded_id(value, max_bytes=192):
    return (isinstance(value, str)
            and 1 <= len(value.encode('utf-8')) <= max_bytes
            and bool(ID_RE.match(value)))


def _bounded_text(value, max_chars):
    if not isinstance(value, str) or not value.strip() or len(value) > max_chars:
        return False
    return not any(ord(c) < 32 and c not in ('\n', '\t') for c in value)


def _check_refs(refs, admitted, max_len, min_len=0, where='refs'):
    if not isinstance(refs, list) or not (min_len <= len(refs) <= max_len):
        raise AdmissionError(f'{where}: list length outside {min_len}..{max_len}')
    if len(set(refs)) != len(refs):
        raise AdmissionError(f'{where}: duplicate reference')
    for ref in refs:
        if not _bounded_id(ref) or ref not in admitted:
            raise AdmissionError(f'{where}: reference outside admitted domain context: {ref!r}')


def _check_outcomes(items, admitted, where):
    if not isinstance(items, list) or len(items) > 8:
        raise AdmissionError(f'{where}: outcome list exceeds 8')
    for idx, item in enumerate(items):
        if not isinstance(item, dict) or set(item) != OUTCOME_KEYS:
            raise AdmissionError(f'{where}[{idx}]: outcome keys mismatch')
        if not _bounded_text(item['description'], 500) or not _bounded_text(item['horizon'], 500):
            raise AdmissionError(f'{where}[{idx}]: invalid description or horizon')
        _check_refs(item['evidence_refs'], admitted, 16, 0, f'{where}[{idx}].evidence_refs')


def validate_analysis(analysis, admitted_refs, admitted_objectives=None):
    """Validate a StrategicAnalysis dict against admitted domain evidence references."""
    if not isinstance(analysis, dict) or set(analysis) != ANALYSIS_KEYS:
        raise AdmissionError('analysis keys mismatch')
    admitted = set(admitted_refs)
    objectives = analysis['user_objectives']
    if not isinstance(objectives, list) or not (1 <= len(objectives) <= 8):
        raise AdmissionError('user_objectives must contain 1..8 records')
    seen_obj = set()
    for idx, obj in enumerate(objectives):
        if not isinstance(obj, dict) or set(obj) != OBJECTIVE_KEYS:
            raise AdmissionError(f'user_objectives[{idx}]: keys mismatch')
        if not _bounded_id(obj['objective_ref']) or obj['objective_ref'] in seen_obj:
            raise AdmissionError(f'user_objectives[{idx}]: invalid or duplicate objective_ref')
        seen_obj.add(obj['objective_ref'])
        if admitted_objectives is not None and obj['objective_ref'] not in admitted_objectives:
            raise AdmissionError(f'user_objectives[{idx}]: objective_ref not admitted')
        priority = obj['priority']
        if isinstance(priority, bool) or not isinstance(priority, int) or not (1 <= priority <= 8):
            raise AdmissionError(f'user_objectives[{idx}]: priority must be integer 1..8')
        if not _bounded_text(obj['horizon'], 500) or not _bounded_text(obj['summary'], 500):
            raise AdmissionError(f'user_objectives[{idx}]: invalid horizon or summary')

    alternatives = analysis['alternatives']
    if not isinstance(alternatives, list) or not (1 <= len(alternatives) <= 8):
        raise AdmissionError('alternatives must contain 1..8 records')
    option_ids = set()
    for idx, alt in enumerate(alternatives):
        if not isinstance(alt, dict) or set(alt) != ALTERNATIVE_KEYS:
            raise AdmissionError(f'alternatives[{idx}]: keys mismatch')
        oid = alt['option_id']
        if not _bounded_id(oid) or oid in option_ids:
            raise AdmissionError(f'alternatives[{idx}]: invalid or duplicate option_id')
        option_ids.add(oid)
        if not _bounded_text(alt['action_summary'], 1200):
            raise AdmissionError(f'alternatives[{idx}]: invalid action_summary')
        _check_refs(alt['supporting_refs'], admitted, 16, 0, f'alternatives[{idx}].supporting_refs')
        uncertainties = alt['uncertainties']
        if not isinstance(uncertainties, list) or len(uncertainties) > 8:
            raise AdmissionError(f'alternatives[{idx}]: uncertainties exceeds 8')
        if any(not _bounded_text(u, 500) for u in uncertainties):
            raise AdmissionError(f'alternatives[{idx}]: invalid uncertainty text')
        _check_outcomes(alt['consequences'], admitted, f'alternatives[{idx}].consequences')
        _check_outcomes(alt['relational_costs'], admitted, f'alternatives[{idx}].relational_costs')

    selected = analysis['selected_option_id']
    if selected is not None and (not isinstance(selected, str) or selected not in option_ids):
        raise AdmissionError('selected_option_id must be None or an existing option_id')
    if not _bounded_text(analysis['justification'], 1200):
        raise AdmissionError('invalid justification')

    unc = analysis['uncertainty']
    if not isinstance(unc, dict) or set(unc) != UNCERTAINTY_KEYS:
        raise AdmissionError('uncertainty keys mismatch')
    for key in ('missing_information', 'competing_explanations'):
        items = unc[key]
        if not isinstance(items, list) or len(items) > 8 or any(not _bounded_text(x, 500) for x in items):
            raise AdmissionError(f'uncertainty.{key}: invalid list')
    forecasts = unc['forecasts']
    if not isinstance(forecasts, list) or len(forecasts) > 8:
        raise AdmissionError('uncertainty.forecasts exceeds 8')
    for idx, fc in enumerate(forecasts):
        if not isinstance(fc, dict) or set(fc) != FORECAST_KEYS:
            raise AdmissionError(f'forecasts[{idx}]: keys mismatch')
        prob = fc['probability_permille']
        if isinstance(prob, bool) or not isinstance(prob, int) or not (0 <= prob <= 1000):
            raise AdmissionError(f'forecasts[{idx}]: probability_permille must be int 0..1000')
        if not _bounded_text(fc['event'], 500) or not _bounded_text(fc['resolution_condition'], 500):
            raise AdmissionError(f'forecasts[{idx}]: invalid event or resolution_condition')
        _check_refs(fc['evidence_refs'], admitted, 16, 0, f'forecasts[{idx}].evidence_refs')

    revisions = analysis['revision_conditions']
    if (not isinstance(revisions, list) or not (1 <= len(revisions) <= 8)
            or any(not _bounded_text(r, 500) for r in revisions)):
        raise AdmissionError('revision_conditions must be 1..8 bounded strings')
    return analysis


def build_envelope(
    analysis,
    *,
    decision_id,
    task_id,
    domain,
    produced_at_ms,
    expires_at_ms,
    source_run_ref,
    objective_refs,
    evidence_refs,
    admitted_refs,
    max_evidence_expiry_ms=None,
):
    """Construct and validate a host-authored wanda.decision_v1 envelope."""
    effective_expiry = expires_at_ms
    if max_evidence_expiry_ms is not None:
        effective_expiry = min(effective_expiry, max_evidence_expiry_ms)
    envelope = {
        'schema_version': SCHEMA_VERSION,
        'kind': KIND,
        'decision_id': decision_id,
        'task_id': task_id,
        'domain': domain,
        'produced_at_ms': produced_at_ms,
        'expires_at_ms': effective_expiry,
        'source_run_ref': source_run_ref,
        'objective_refs': list(objective_refs),
        'evidence_refs': list(evidence_refs),
        'analysis': analysis,
    }
    validate_envelope(envelope, expected_domain=domain, admitted_refs=admitted_refs)
    return envelope


def validate_envelope(envelope, *, expected_domain, admitted_refs, now_ms=None):
    """Validate a complete wanda.decision_v1 envelope against domain and size ceilings."""
    if not isinstance(envelope, dict) or set(envelope) != ENVELOPE_KEYS:
        raise AdmissionError('decision_v1 envelope keys mismatch')
    if envelope['schema_version'] != SCHEMA_VERSION or envelope['kind'] != KIND:
        raise AdmissionError('unsupported decision_v1 schema_version or kind')
    if not _bounded_id(envelope['decision_id'], 192):
        raise AdmissionError('invalid decision_id')
    if not _bounded_id(envelope['task_id'], 128):
        raise AdmissionError('invalid task_id (must be <= 128 bytes)')
    if not _bounded_id(envelope['domain'], 192) or envelope['domain'] != expected_domain:
        raise AdmissionError('decision_v1 domain mismatch')
    if not _bounded_id(envelope['source_run_ref'], 192):
        raise AdmissionError('invalid source_run_ref')
    prod, exp = envelope['produced_at_ms'], envelope['expires_at_ms']
    if (isinstance(prod, bool) or isinstance(exp, bool)
            or not isinstance(prod, int) or not isinstance(exp, int)):
        raise AdmissionError('timestamps must be 64-bit integers')
    if prod <= 0 or exp <= prod or exp - prod > MAX_HORIZON_MS:
        raise AdmissionError('expires_at_ms must be > produced_at_ms and within 24 hours')
    if now_ms is not None and exp <= now_ms:
        raise AdmissionError('decision_v1 envelope is expired')

    obj_refs = envelope['objective_refs']
    if not isinstance(obj_refs, list) or len(obj_refs) > 8 or len(set(obj_refs)) != len(obj_refs):
        raise AdmissionError('objective_refs must be <= 8 distinct ids')
    if any(not _bounded_id(r, 192) for r in obj_refs):
        raise AdmissionError('invalid objective_ref')

    admitted = set(admitted_refs)
    _check_refs(envelope['evidence_refs'], admitted, 64, 1, 'evidence_refs')
    admitted_subset = set(envelope['evidence_refs'])
    validate_analysis(
        envelope['analysis'],
        admitted_subset,
        admitted_objectives=set(obj_refs) if obj_refs else None,
    )
    encoded = json.dumps(envelope, ensure_ascii=False, separators=(',', ':')).encode('utf-8')
    if len(encoded) > MAX_ENVELOPE_BYTES:
        raise AdmissionError(f'decision_v1 exceeds {MAX_ENVELOPE_BYTES} UTF-8 bytes ({len(encoded)})')
    return envelope


def build_correlation(envelope, *, proposal_id=None, hypothesis_id=None, recommendation_ids=()):
    """Build the companion host-authored correlation record outside ModelOutput."""
    if proposal_id is not None and not _bounded_id(proposal_id, 192):
        raise AdmissionError('invalid proposal_id')
    if hypothesis_id is not None and not _bounded_id(hypothesis_id, 192):
        raise AdmissionError('invalid hypothesis_id')
    rec_ids = list(recommendation_ids)
    if len(rec_ids) > 12 or len(set(rec_ids)) != len(rec_ids) or any(not _bounded_id(r, 192) for r in rec_ids):
        raise AdmissionError('invalid recommendation_ids')
    return {
        'schema_version': SCHEMA_VERSION,
        'decision_id': envelope['decision_id'],
        'decision_ref': f'{envelope["domain"]}/wanda/decisions/{envelope["decision_id"]}',
        'decision_sha256': digest(envelope),
        'task_id': envelope['task_id'],
        'domain': envelope['domain'],
        'proposal_id': proposal_id,
        'hypothesis_id': hypothesis_id,
        'recommendation_ids': rec_ids,
        'source_run_ref': envelope['source_run_ref'],
        'produced_at_ms': envelope['produced_at_ms'],
        'expires_at_ms': envelope['expires_at_ms'],
    }


def verify_correlation(envelope, correlation):
    """Verify that a correlation record matches a validated decision_v1 envelope."""
    expected = build_correlation(
        envelope,
        proposal_id=correlation.get('proposal_id'),
        hypothesis_id=correlation.get('hypothesis_id'),
        recommendation_ids=correlation.get('recommendation_ids', ()),
    )
    if correlation != expected:
        raise AdmissionError('decision correlation mismatch')
    return True


def project_cortex_document_node(envelope):
    """Project an accepted decision_v1 envelope as an inferred Cortex Document node.

    Preserves 'inferred' model-analysis provenance and never counts as an independent
    corroborating evidence vote alongside the original cited messages.
    """
    decision_ref = f'{envelope["domain"]}/wanda/decisions/{envelope["decision_id"]}'
    selected = envelope['analysis']['selected_option_id']
    choice = f'selected={selected}' if selected else 'selected=none'
    summary = f'[inferred model analysis; {choice}] {envelope["analysis"]["justification"]}'[:500]
    return {
        'node_id': f'doc:{envelope["decision_id"]}',
        'kind': 'document',
        'domain': envelope['domain'],
        'provenance_ref': decision_ref,
        'standing': 'inferred',
        'independent_evidence_vote': False,
        'cited_evidence_refs': list(envelope['evidence_refs']),
        'summary': summary,
        'produced_at_ms': envelope['produced_at_ms'],
        'expires_at_ms': envelope['expires_at_ms'],
    }
