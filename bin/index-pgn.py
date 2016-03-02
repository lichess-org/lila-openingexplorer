#!/usr/bin/env python3

import requests
import sys
import time

if sys.version_info[0] < 3:
    sys.exit("Minimum requirement is Python 3.3.")
elif sys.version_info[0] == 3 and sys.version_info[1] <= 2:
    sys.exit("Minimum requirement is Python 3.3.")
elif sys.version_info[0] == 3 and sys.version_info[1] >= 3:
    pass
elif sys.version_info[0] >= 4:
    pass
else:
    sys.exit("Oh dear, you appear to have broken reality ... how did you manage that anyway?")

if len(sys.argv) <= 2:
    sys.exit("Usage: index-pgn.py <master|lichess/standard|...> <pgn-file>")

endpoint = sys.argv[1]

f = open(sys.argv[2], encoding="utf-8-sig", errors="ignore")

def split_pgn(f):
    buf = []

    got_header = False

    for line in f:
        buf.append(line.strip())

        if not line.strip() and got_header:
            got_header = False
        elif not line.strip() and not got_header:
            pgn = "\n".join(buf).strip()
            if pgn:
                yield pgn

            buf[:] = []
        elif line.startswith("[Event"):
            got_header = True

    pgn = "\n".join(buf).strip()
    if pgn:
        yield pgn

def has_rating(pgn):
    return "[WhiteElo" in pgn or "[BlackElo" in pgn

t = time.time()

def send(buf):
    global t

    res = requests.put("http://localhost:9000/import/" + endpoint, data=buf)

    new_t = time.time()

    print("HTTP %d: server: %s, wallclock: %.01f ms" % (res.status_code, res.text, (new_t - t) * 1000))
    if res.status_code != 200:
        print(buf)
        print(res.text)

    t = new_t

for buf in (pgn for pgn in split_pgn(f) if has_rating(pgn)):
    send(buf)
