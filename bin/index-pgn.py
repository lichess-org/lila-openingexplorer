#!/usr/bin/env python

import requests
import random
import sys
import itertools
import time
import re

if len(sys.argv) != 3:
    sys.exit("Usage: python3 index-pgn.py <master|lichess/standard|...> <pgn-file>")

endpoint = sys.argv[1]

f = open(sys.argv[2], encoding="utf-8", errors="ignore")

c = itertools.count(1)

buf = ""
got_header = False

rating_regex = re.compile("\[(White|Black)Elo ", re.MULTILINE)

def send(buf):
    if rating_regex.search(buf):
        t = time.time()
        res = requests.put("http://localhost:9000/import/" + endpoint, data=buf.encode("utf-8"))
        print("[%d, %.01fms] HTTP %d: %s" % (next(c), (time.time() - t) * 1000, res.status_code, res.text))
        if res.status_code != 200:
            print(buf)
    else:
        next(c)

for line in f:
    buf += line
    if not line.strip() and got_header:
        got_header = False
    elif not line.strip() and not got_header:
        send(buf)
        buf = ""
    elif line.startswith("[Event"):
        got_header = True

if buf.strip():
    send(buf)
