#!/usr/bin/env python3
"""Challenge VM completion and gate identity independently of the VM controller."""
import copy
import runpy
from pathlib import Path
import unittest

vm = runpy.run_path(str(Path(__file__).with_name('check-linux-vm.py')))


class Completion(unittest.TestCase):
    def test_success_and_real_failure_keep_their_status(self):
        for status in (0, 23, 143):
            text = f'boot log\nVERIFICATION_RESULT exit={status}\nVERIFICATION_STOPPED\n'
            self.assertEqual(vm['guest_result'](text), status)

    def test_shutdown_or_zero_alone_cannot_pass(self):
        for text in ('', 'VERIFICATION_STOPPED\n', 'VERIFICATION_RESULT exit=0\n',
                     'VERIFICATION_STOPPED\nVERIFICATION_RESULT exit=0\n'):
            with self.subTest(text=text), self.assertRaises(ValueError):
                vm['guest_result'](text)

    def test_duplicate_malformed_and_cleanup_failure_cannot_pass(self):
        good = 'VERIFICATION_RESULT exit=0\nVERIFICATION_STOPPED\n'
        for text in (good + good, good + 'VERIFICATION_ERROR operation=unmount\n',
                     good.replace('exit=0', 'exit=-1'), good.replace('exit=0', 'exit=256'),
                     good.replace('exit=0', 'exit=0 garbage')):
            with self.subTest(text=text), self.assertRaises(ValueError):
                vm['guest_result'](text)

    def test_full_gate_requires_source_identity_and_cleanup(self):
        good = dict(status='passed', scope='full', source_sha256='expected',
                    inputs_unchanged=True, finalization_errors=[], stages=[dict(status='passed')])
        vm['validate_gate'](good, 'expected')
        for name, value in (('status', 'failed'), ('scope', 'core'),
                            ('source_sha256', 'changed'), ('inputs_unchanged', False),
                            ('finalization_errors', ['cleanup failed']), ('stages', []),
                            ('stages', [dict(status='failed')])):
            bad = copy.deepcopy(good)
            bad[name] = value
            with self.subTest(name=name, value=value), self.assertRaises(ValueError):
                vm['validate_gate'](bad, 'expected')
        with self.assertRaises(ValueError):
            vm['validate_gate'](None, 'expected')


if __name__ == '__main__':
    unittest.main()
