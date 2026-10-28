"""Transaction replay checks: failed realloc, moved-address reuse and thresholds."""
import unittest
import json
from pathlib import Path
import tempfile
from analyze_allocation_trace import Replay, analyze

M = 1024 * 1024


def event(sequence, kind, pointer, size=0, new_size=0, transaction=0, frames=(10,)):
    return dict(sequence=sequence, kind=kind, pointer=pointer, size=size,
                new_size=new_size, transaction=transaction, frames=frames)


class ReplayTests(unittest.TestCase):
    def test_failed_realloc_restores_original_site_and_bytes(self):
        replay = Replay(M)
        for e in [event(1, 1, 100, M), event(2, 3, 100, M, M*2, frames=(20,)),
                  event(3, 4, 0, transaction=2)]:
            replay.apply(e)
        self.assertEqual(replay.live, {100: (M, (10,))})

    def test_old_address_reused_before_moved_realloc_returns_is_preserved(self):
        replay = Replay(M)
        for e in [event(1, 1, 100, M), event(2, 3, 100, M, M*2, frames=(20,)),
                  event(3, 1, 100, M*3, frames=(30,)), event(4, 4, 200, transaction=2)]:
            replay.apply(e)
        self.assertEqual(replay.live, {100: (M*3, (30,)), 200: (M*2, (20,))})

    def test_crossing_threshold_and_shrinking_remove_old_ownership(self):
        replay = Replay(M)
        for e in [event(1, 3, 100, M//2, M*2), event(2, 4, 200, transaction=1),
                  event(3, 3, 200, M*2, M//2), event(4, 4, 200, transaction=3)]:
            replay.apply(e)
        self.assertEqual(replay.live, {})
        self.assertEqual(replay.pending, {})

    def test_duplicate_and_unmatched_transaction_are_errors(self):
        replay = Replay(M)
        replay.apply(event(1, 1, 100, M))
        with self.assertRaises(ValueError): replay.apply(event(2, 1, 100, M))
        with self.assertRaises(ValueError): replay.apply(event(3, 4, 200, transaction=99))

    def test_overflow_and_incomplete_transactions_refuse_attribution(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'trace.json'
            trace = dict(minimum_bytes=M, dropped_events=1, recorded_events=0, events=[],
                         symbols={}, trace_storage_bytes=1024, image_base=0)
            path.write_text(json.dumps(trace), encoding='utf-8')
            with self.assertRaisesRegex(ValueError, 'overflow'): analyze(path)
            trace.update(dropped_events=0, recorded_events=1,
                         events=[event(1, 3, 100, M, M*2)])
            path.write_text(json.dumps(trace), encoding='utf-8')
            with self.assertRaisesRegex(ValueError, 'incomplete'): analyze(path)

    def test_mismatched_layout_is_not_treated_as_retention(self):
        for kind in (2, 3):
            replay = Replay(M)
            replay.apply(event(1, 1, 100, M))
            with self.assertRaisesRegex(ValueError, 'layout'):
                replay.apply(event(2, kind, 100, M*2, M*3))


if __name__ == "__main__":
    unittest.main()
