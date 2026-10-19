"""Shared behavioural predicates for live harness observations."""
import re


def well_formed_version(value):
    number = r"(?:0|[1-9][0-9]*)"
    identifier = r"[0-9A-Za-z-]+"
    match = isinstance(value, str) and re.fullmatch(
        rf"{number}\.{number}\.{number}(?:-({identifier}(?:\.{identifier})*))?"
        rf"(?:\+{identifier}(?:\.{identifier})*)?", value)
    return bool(match) and (match[1] is None or all(
        not part.isdigit() or part == "0" or not part.startswith("0")
        for part in match[1].split(".")))


def capability_catalogue(document):
    section = document.split("## Capability catalogue", 1)[1].split("\n## ", 1)[0]
    table = {cap for line in section.splitlines() if line.startswith("|")
             for cap in re.findall(r"`([^`]+)`", line.split("|")[1])}
    # Some implemented capabilities are declared in the operation addenda.
    additions = set(re.findall(r"^Capability `([^`]+)` adds\b", document, re.M))
    return table | additions


def check_capabilities(required, live, catalogue):
    for name, values in (("fixture", required), ("live", live)):
        if not isinstance(values, list) or not all(isinstance(cap, str) for cap in values):
            raise ValueError(f"{name} caps must be an array of names")
    missing = set(required) - set(live)
    unknown = set(live) - set(catalogue)
    if missing or unknown:
        raise ValueError(f"missing fixture caps={sorted(missing)}; unknown live caps={sorted(unknown)}")


def diagnostic_lines(stderr):
    return [re.sub(r"\x1b\[[0-9;]*m", "", line).strip()
            for line in stderr.splitlines() if line.strip()]


def healthy_diagnostics(stderr):
    """New INFO/DEBUG diagnostics are fine; warnings and runtime failures are not."""
    for line in diagnostic_lines(stderr):
        if re.search(r"\b(?:panic(?:ked)?|backtrace)\b", line, re.I):
            return False
        level = re.search(r"(?:^|\s)(INFO|DEBUG|WARN(?:ING)?|ERROR)\b", line)
        if level is None or level[1] not in ("INFO", "DEBUG"):
            return False
    return True


def ordered_shutdown(stderr, complete=True):
    """Require each owner drain to release the engine before unlinking sockets.

    Engine-only restarts may drain before the owner starts shutting down.
    Unrelated success diagnostics may appear between any two phases.
    """
    phases = [r"daemon draining\b", r"engine drained\b", r"daemon slot released\b",
              r"daemon sockets unlinked\b", r"daemon stopped\b"]
    next_phase = 0
    stopped = False
    for line in diagnostic_lines(stderr):
        phase = next((i for i, pattern in enumerate(phases) if re.search(pattern, line)), None)
        if phase is None or (phase == 1 and next_phase == 0):
            continue
        if phase != next_phase:
            return False
        if phase == 4:
            if re.search(r"\bcode=0(?:\s|$)", line) is None:
                return False
            stopped = True
            next_phase = 0
        else:
            next_phase += 1
    return not complete or (stopped and next_phase == 0)


def expected_daemon_stderr(stderr, complete=True):
    return healthy_diagnostics(stderr) and ordered_shutdown(stderr, complete)
