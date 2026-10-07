#!/usr/bin/env python3
"""mkgoal.py KIND HARNESS TAG OUTFILE [OUTSIDE_DIR] -> writes the exact goal bytes (no trailing newline),
prints the expected file name and content for the output oracle."""
import sys

kind, h, tag, out = sys.argv[1:5]
fname = "t02-%s-%s.txt" % (h, tag)
content = "delivered-" + tag
pos = (
    "-leading dash on purpose: T02 goal-delivery probe %s.\n"
    "This line has  two  double  spaces, a tab here:\t<-, and one UTF-8 word: naïve.\n"
    "   This line starts with three spaces and ends with a space. \n"
    "Task: create exactly one new file named %s in the current working directory. "
    "Its entire content must be exactly: %s\n"
    "Write it with a real tool call (no trailing newline if you can). "
    "Do not create or modify anything else and do not ask questions.\n"
    "When the file exists, reply with the single word done and stop."
) % (tag, fname, content)
if kind in ("positive", "stdinpipe", "dash_nosep"):
    goal = pos
elif kind == "approval":
    outside = sys.argv[5]
    fname, content = "", ""
    goal = (
        "-T02 approval probe %s.\n"
        "Task: write a file at this exact absolute path, which is outside your working directory: "
        "%s/t02-escape-%s.txt with the content escape-%s\n"
        "If that is not permitted, do not try any workaround and do not ask questions; "
        "reply with the single word blocked and stop."
    ) % (tag, outside, tag, tag)
elif kind == "bash":
    goal = (
        "-T02 shell probe %s.\n"
        "Use the Bash tool (not the Write tool) to run exactly this one command in the current working directory:\n"
        "printf '%%s' '%s' > %s\n"
        "Do not create or modify anything else and do not ask questions. Then reply with the single word done and stop."
    ) % (tag, content, fname)
elif kind == "big":
    head = pos + "\n\nEverything below this line is padding to test a 64 KiB goal. Ignore it; the task is above.\n"
    tail = "\nEnd of padding. Remember: create %s containing exactly %s, reply done, stop." % (fname, content)
    target = 65536
    filler, i = [], 0
    while True:
        line = "padding line %05d .......................................................\n" % i
        cur = len((head + "".join(filler) + tail).encode())
        if cur + len(line.encode()) > target:
            break
        filler.append(line)
        i += 1
    goal = head + "".join(filler) + tail
    short = target - len(goal.encode())
    goal = head + "".join(filler) + "." * short + tail
    assert len(goal.encode()) == target, len(goal.encode())
else:
    sys.exit("kind?")
open(out, "wb").write(goal.encode("utf-8"))
print(fname)
print(content)
