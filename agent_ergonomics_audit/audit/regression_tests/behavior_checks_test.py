#!/usr/bin/env python3
"""Positive and negative controls for harness behaviour checks."""
import unittest

from behavior_checks import (capability_catalogue, check_capabilities, expected_daemon_stderr,
                             healthy_diagnostics, well_formed_version)


class BehaviorChecks(unittest.TestCase):
    phases = ["INFO daemon draining", "INFO engine drained in 42 ms",
              "INFO daemon slot released elapsed_ms=43", "INFO daemon sockets unlinked",
              "INFO daemon stopped reason=Quit code=0"]

    def test_new_success_diagnostics_are_allowed(self):
        for line in ["INFO new subsystem started", "DEBUG new detail key=value", "INFO diagnostic mentions WARN and ERROR",
                     "2026-10-19T12:00:00Z INFO new detail", "\x1b[32mINFO\x1b[0m new detail"]:
            with self.subTest(line=line):
                self.assertTrue(healthy_diagnostics(line))
                self.assertTrue(expected_daemon_stderr("\n".join([line, *self.phases, line])))

    def test_failures_are_rejected_even_in_success_logs(self):
        for line in ["WARN failed", "WARNING failed", "ERROR failed",
                     "thread 'worker' panicked at x", "stack backtrace:",
                     "INFO panic caught", "DEBUG backtrace: worker", "unstructured failure"]:
            with self.subTest(line=line):
                self.assertFalse(expected_daemon_stderr("\n".join([*self.phases, line])))
                self.assertFalse(expected_daemon_stderr(line, complete=False))

    def test_every_drain_phase_is_required(self):
        for index in range(len(self.phases)):
            with self.subTest(missing=index):
                self.assertFalse(expected_daemon_stderr("\n".join(self.phases[:index] + self.phases[index+1:])))
        self.assertFalse(expected_daemon_stderr("INFO startup only"))

    def test_parked_shutdown_proves_session_absence_in_the_drain_phase(self):
        parked = self.phases.copy()
        parked[1] = "INFO engine slot empty state=parked session=absent"
        self.assertTrue(expected_daemon_stderr("\n".join(parked)))
        for line in ("INFO parked", "INFO engine slot empty state=running session=absent",
                     "INFO engine slot empty state=parked", "INFO engine slot released"):
            invalid = parked.copy(); invalid[1] = line
            self.assertFalse(expected_daemon_stderr("\n".join(invalid)))
        for index in range(len(parked)):
            self.assertFalse(expected_daemon_stderr("\n".join(parked[:index]+parked[index+1:])))
        parked[0], parked[1] = parked[1], parked[0]
        self.assertFalse(expected_daemon_stderr("\n".join(parked)))

    def test_every_drain_phase_keeps_its_order(self):
        for index in range(len(self.phases)-1):
            phases = self.phases.copy()
            phases[index], phases[index+1] = phases[index+1], phases[index]
            with self.subTest(swapped=index):
                self.assertFalse(expected_daemon_stderr("\n".join(phases)))

    def test_restarts_and_additional_success_lines_do_not_hide_a_bad_drain(self):
        before = "INFO engine drained in 3 ms\nINFO engine starting session\n"
        complete = "\nDEBUG another success detail\n".join(self.phases)
        self.assertTrue(expected_daemon_stderr(before + complete))
        self.assertTrue(expected_daemon_stderr(complete + "\n" + complete))
        self.assertFalse(expected_daemon_stderr(complete + "\nINFO daemon draining"))
        self.assertFalse(expected_daemon_stderr(complete.replace("code=0", "code=70")))
        self.assertFalse(expected_daemon_stderr(complete + "\nINFO daemon sockets unlinked"))

    def test_running_owner_does_not_require_a_shutdown(self):
        self.assertTrue(expected_daemon_stderr("INFO new activity", complete=False))
        self.assertTrue(expected_daemon_stderr("INFO daemon draining", complete=False))
        self.assertFalse(expected_daemon_stderr("INFO daemon sockets unlinked", complete=False))

    def test_catalogue_includes_table_and_explicit_operation_addenda(self):
        document = "## Capability catalogue\n| `files`, `torrents` | operations |\n## Additions\nCapability `free_space` adds read-only space checks.\nIncidental `invented` text."
        self.assertEqual(capability_catalogue(document), {"files", "torrents", "free_space"})

    def test_capability_order_and_known_additions_are_allowed(self):
        check_capabilities(["files", "torrents"], ["torrents", "watch", "files"],
                           {"files", "torrents", "watch"})

    def test_missing_unknown_or_malformed_caps_fail(self):
        for live in [["files"], ["torrents", "invented"], "torrents", [None]]:
            with self.subTest(live=live), self.assertRaises(ValueError):
                check_capabilities(["torrents"], live, {"files", "torrents"})

    def test_version_checks_shape_without_pinning_patch(self):
        for version in ["0.0.0", "1.2.30", "2.0.0", "1.2.3-rc.1+build.7"]:
            with self.subTest(version=version):
                self.assertTrue(well_formed_version(version))
        for version in [None, 123, "", "1.2", "v1.2.3", "01.2.3", "1.2.-3", "1.2.3-01", "1.2.3 junk"]:
            with self.subTest(version=version):
                self.assertFalse(well_formed_version(version))


if __name__ == "__main__":
    unittest.main()
