import copy
import unittest
from archie_wanda.contracts import AdmissionError
from archie_wanda.decision import (
    build_correlation,
    build_envelope,
    project_cortex_document_node,
    validate_analysis,
    validate_envelope,
    verify_correlation,
)


def _sample_analysis():
    return {
        'user_objectives': [
            {
                'objective_ref': 'obj:trip-coordination',
                'priority': 1,
                'horizon': '48h',
                'summary': 'Ensure everyone knows the trip moved to July before booking flights.',
            }
        ],
        'alternatives': [
            {
                'option_id': 'opt-direct-ping',
                'action_summary': 'Send Alice a direct update noting the trip moved from May to July.',
                'supporting_refs': ['personal/ev-001', 'personal/ev-002'],
                'uncertainties': ['Whether Alice has already checked May flight prices.'],
                'consequences': [
                    {
                        'description': 'Alice sees the July dates before purchasing tickets.',
                        'horizon': 'immediate',
                        'evidence_refs': ['personal/ev-001'],
                    }
                ],
                'relational_costs': [
                    {
                        'description': 'Minimal interruption via a brief one-line heads-up.',
                        'horizon': 'immediate',
                        'evidence_refs': ['personal/ev-002'],
                    }
                ],
            },
            {
                'option_id': 'opt-wait',
                'action_summary': 'Wait until tomorrow morning to post a reminder in the group chat.',
                'supporting_refs': ['personal/ev-002'],
                'uncertainties': ['Alice may book May tickets tonight.'],
                'consequences': [
                    {
                        'description': 'Risk of booking mismatch if Alice acts tonight.',
                        'horizon': '24h',
                        'evidence_refs': ['personal/ev-001'],
                    }
                ],
                'relational_costs': [],
            },
        ],
        'selected_option_id': 'opt-direct-ping',
        'justification': 'Alice was absent when July was announced in personal/ev-002 and last saw May in personal/ev-001.',
        'uncertainty': {
            'missing_information': ['Whether Bruno already messaged Alice privately.'],
            'competing_explanations': ['Alice might have muted the group chat temporarily.'],
            'forecasts': [
                {
                    'event': 'Alice confirms the July schedule once messaged directly.',
                    'probability_permille': 850,
                    'resolution_condition': 'Alice replies acknowledging July.',
                    'evidence_refs': ['personal/ev-001', 'personal/ev-002'],
                }
            ],
        },
        'revision_conditions': [
            'Alice posts in the group chat acknowledging the July update.',
            'The group changes the trip month again.',
        ],
    }


class DecisionV1Tests(unittest.TestCase):
    def setUp(self):
        self.admitted_refs = {'personal/ev-001', 'personal/ev-002', 'personal/ev-003'}

    def test_round_trip_envelope_correlation_and_cortex_document_projection(self):
        analysis = validate_analysis(_sample_analysis(), self.admitted_refs)
        env = build_envelope(
            analysis,
            decision_id='dec-2026-0001',
            task_id='task-042',
            domain='personal',
            produced_at_ms=1_700_000_000_000,
            expires_at_ms=1_700_086_400_000,
            source_run_ref='run:wanda-bonsai2-9b',
            objective_refs=['obj:trip-coordination'],
            evidence_refs=['personal/ev-001', 'personal/ev-002'],
            admitted_refs=self.admitted_refs,
            max_evidence_expiry_ms=1_700_030_000_000,
        )
        self.assertEqual(env['schema_version'], 1)
        self.assertEqual(env['kind'], 'wanda.decision_v1')
        self.assertEqual(env['expires_at_ms'], 1_700_030_000_000)

        corr = build_correlation(
            env,
            proposal_id='prop-1',
            hypothesis_id='hyp-1',
            recommendation_ids=['rec-1'],
        )
        self.assertTrue(verify_correlation(env, corr))
        tampered = dict(corr, decision_sha256='0' * 64)
        with self.assertRaises(AdmissionError):
            verify_correlation(env, tampered)

        node = project_cortex_document_node(env)
        self.assertEqual(node['kind'], 'document')
        self.assertEqual(node['standing'], 'inferred')
        self.assertFalse(node['independent_evidence_vote'])
        self.assertEqual(node['provenance_ref'], 'personal/wanda/decisions/dec-2026-0001')

    def test_rejects_unadmitted_refs_unknown_keys_and_invalid_options(self):
        bad_ref = _sample_analysis()
        bad_ref['alternatives'][0]['supporting_refs'] = ['personal/forged-999']
        with self.assertRaises(AdmissionError):
            validate_analysis(bad_ref, self.admitted_refs)

        extra_key = _sample_analysis()
        extra_key['authority_override'] = True
        with self.assertRaises(AdmissionError):
            validate_analysis(extra_key, self.admitted_refs)

        bad_sel = _sample_analysis()
        bad_sel['selected_option_id'] = 'opt-nonexistent'
        with self.assertRaises(AdmissionError):
            validate_analysis(bad_sel, self.admitted_refs)

        bad_prob = _sample_analysis()
        bad_prob['uncertainty']['forecasts'][0]['probability_permille'] = 1200
        with self.assertRaises(AdmissionError):
            validate_analysis(bad_prob, self.admitted_refs)

    def test_rejects_domain_mismatch_expired_horizon_and_oversized_envelope(self):
        analysis = _sample_analysis()
        env = build_envelope(
            analysis,
            decision_id='dec-2026-0002',
            task_id='task-042',
            domain='personal',
            produced_at_ms=1_700_000_000_000,
            expires_at_ms=1_700_036_000_000,
            source_run_ref='run:wanda-bonsai2-9b',
            objective_refs=['obj:trip-coordination'],
            evidence_refs=['personal/ev-001', 'personal/ev-002'],
            admitted_refs=self.admitted_refs,
        )
        with self.assertRaises(AdmissionError):
            validate_envelope(env, expected_domain='work', admitted_refs=self.admitted_refs)

        with self.assertRaises(AdmissionError):
            validate_envelope(
                env,
                expected_domain='personal',
                admitted_refs=self.admitted_refs,
                now_ms=1_700_040_000_000,
            )

        oversized = copy.deepcopy(analysis)
        oversized['alternatives'] = [
            {
                'option_id': f'opt-{i}',
                'action_summary': 'A' * 1150,
                'supporting_refs': ['personal/ev-001'],
                'uncertainties': ['U' * 450 for _ in range(4)],
                'consequences': [],
                'relational_costs': [],
            }
            for i in range(8)
        ]
        oversized['selected_option_id'] = 'opt-0'
        with self.assertRaises(AdmissionError):
            build_envelope(
                oversized,
                decision_id='dec-big',
                task_id='task-042',
                domain='personal',
                produced_at_ms=1_700_000_000_000,
                expires_at_ms=1_700_036_000_000,
                source_run_ref='run:wanda-bonsai2-9b',
                objective_refs=['obj:trip-coordination'],
                evidence_refs=['personal/ev-001', 'personal/ev-002'],
                admitted_refs=self.admitted_refs,
            )
