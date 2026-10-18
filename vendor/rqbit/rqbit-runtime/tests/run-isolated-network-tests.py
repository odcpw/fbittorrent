#!/usr/bin/env python3
"""Run the real-socket binding fixtures without touching host networking.

Pass one or more built unit-test executables, optionally --measure for the
ignored throughput measurement. Build with Cargo outside the namespace first.
"""
import os
import subprocess
import sys

measure = "--measure" in sys.argv[1:]
executables = [argument for argument in sys.argv[1:] if argument != "--measure"]
assert executables, "pass built rqbit-runtime / dualstack unit-test executables"
host_namespace = os.readlink("/proc/self/ns/net")
for executable in executables:
    arguments = ["isolated_binding_throughput_measurement", "--ignored"] if measure else ["isolated_"]
    command = ["unshare", "-Urn", "env", "FBT_TEST_NETNS=1", f"FBT_TEST_HOST_NETNS={host_namespace}",
               os.path.abspath(executable), *arguments, "--nocapture", "--test-threads=1"]
    subprocess.run(command, check=True)
