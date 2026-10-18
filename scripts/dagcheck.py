#!/usr/bin/env python3
"""Review PLAN §9.3 against §9.4: edges, consumers, levels and critical paths (not a CI gate)."""
import argparse
from pathlib import Path
import re
import sys

ID = r"(?:P\d+-\d{2}|M\d+a?)"


def ids(text):
    def expand(match):
        phase, first, last = match.groups()
        return ", ".join(f"P{phase}-{n:02}" for n in range(int(first), int(last) + 1))
    text = re.sub(r"P(\d+)-(\d{2})\s+to\s+P\1-(\d{2})", expand, text)
    return re.findall(rf"\b{ID}\b", text)


def check(text):
    errors = []
    items = text.split("### 9.3 ", 1)[1].split("### 9.4 ", 1)[0]
    graph_text = text.split("### 9.4 ", 1)[1].split("### 9.5 ", 1)[0]
    graph, consumers = {}, {}
    for line in graph_text.splitlines():
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if line.startswith("|") and len(cells) == 3 and re.fullmatch(ID, cells[0]):
            if cells[0] in graph:
                errors.append(f"duplicate adjacency row {cells[0]}")
            graph[cells[0]], consumers[cells[0]] = ids(cells[1]), set(ids(cells[2]))
    declarations = {}
    headings = list(re.finditer(rf"(?m)^\*\*({ID})\b", items))
    for index, match in enumerate(headings):
        block = items[match.start():headings[index + 1].start() if index + 1 < len(headings) else len(items)]
        node = match[1]
        if node not in graph and "superseded" in block.lower():
            continue
        dependency = re.search(r"\*Depends on:\*\s*(.*?)(?:\n\s*\n|\Z)", block, re.S)
        if dependency is None:
            errors.append(f"{node}: no Depends on declaration")
            continue
        # Parentheses carry explanations, not additional dependency edges.
        dependency_text = re.sub(r"\([^()]*\)", "", dependency[1]).split(";", 1)[0]
        size = re.search(r"\(([SML])\)", block.split("\n", 1)[0])
        declarations[node] = (ids(dependency_text), {"S": 1, "M": 2, "L": 4}[size[1]] if size else 0)
    for node in graph.keys() | declarations.keys():
        if node not in graph or node not in declarations:
            errors.append(f"{node}: missing from {'§9.4' if node not in graph else '§9.3'}")
        elif set(graph[node]) != set(declarations[node][0]):
            errors.append(f"{node}: dependency disagreement §9.3={declarations[node][0]} §9.4={graph[node]}")
    reverse = {node: set() for node in graph}
    for node, dependencies in graph.items():
        for dep in dependencies:
            if dep not in graph:
                errors.append(f"{node}: undefined dependency {dep}")
            else:
                reverse[dep].add(node)
    for node in graph:
        if reverse[node] != consumers[node]:
            errors.append(f"{node}: consumed-by disagreement declared={sorted(consumers[node])} actual={sorted(reverse[node])}")
        if node.startswith("P") and not reverse[node]:
            errors.append(f"orphan item {node}")
    levels, visiting, weighted, depth = {}, [], {}, {}

    def visit(node):
        if node in visiting:
            raise ValueError("cycle: " + " -> ".join(visiting + [node]))
        if node in levels:
            return
        visiting.append(node)
        deps = [dep for dep in graph[node] if dep in graph]
        for dep in deps:
            visit(dep)
        levels[node] = max((levels[dep] + 1 for dep in deps), default=0)
        weight = declarations.get(node, ([], 0))[1]
        previous = max((weighted[dep] for dep in deps), key=lambda x: x[0], default=(0, []))
        weighted[node] = (previous[0] + weight, previous[1] + [node])
        previous_depth = max((depth[dep] for dep in deps), key=len, default=[])
        depth[node] = previous_depth + [node]
        visiting.pop()

    try:
        for node in graph:
            visit(node)
    except ValueError as error:
        errors.append(str(error))
    declared_levels = {}
    for line in graph_text.splitlines():
        match = re.match(r"\|\s*(\d+)\s*\|([^|]+)\|", line)
        if match:
            for node in ids(match[2]):
                if node in declared_levels:
                    errors.append(f"duplicate level entry {node}")
                declared_levels[node] = int(match[1])
    for node in declared_levels.keys() - graph.keys():
        errors.append(f"undefined level entry {node}")
    for node, level in levels.items():
        if declared_levels.get(node) != level:
            errors.append(f"LEVEL differs {node}: plan={declared_levels.get(node)} computed={level}")
    report = [f"items in §9.3: {len(declarations)}; rows in §9.4: {len(graph)}; levels: {max(levels.values(), default=-1)+1}"]
    for node in graph:
        if node.startswith("M") and node in weighted:
            weight, path = weighted[node]
            report.append(f"critical path to {node}: weight {weight}: {' -> '.join(path)}; longest by count {len(depth[node])}: {' -> '.join(depth[node])}")
    return report, errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("plan", nargs="?", type=Path,
                        default=Path(__file__).resolve().parents[1] / "docs/plan-2026-10-18/PLAN.md")
    args = parser.parse_args()
    try:
        report, errors = check(args.plan.read_text())
    except (IndexError, OSError) as error:
        print(f"invalid plan: {error}", file=sys.stderr)
        return 1
    print("\n".join(report))
    for error in errors:
        print(error, file=sys.stderr)
    if not errors:
        print("no undefined IDs, cycles, orphan items, consumer or level disagreements")
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main())
