#!/usr/bin/env python3

import requests
import sys
import time
import re

if len(sys.argv) != 3:
    sys.exit("Usage: python3 index-pgn.py <master|lichess> <pgn-file>")

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

def has_10_moves(pgn, _re=re.compile(r"10\.([a-h]|N|B|R|Q|K|\s)")):
    return bool(_re.search(pgn))

t = time.time()

def send(game_no, buf):
    global t

    res = requests.put("http://localhost:9000/import/" + endpoint, data=buf)

    new_t = time.time()

    print("game: %d http %d: elapsed: %.01f ms" % (game_no, res.status_code, (new_t - t) * 1000))
    if res.status_code != 200:
        print(buf)
        try:
            print(res.text)
        except:
            print("--> decode error!")

    t = new_t

for game_no, buf in enumerate(pgn for pgn in split_pgn(f) if has_rating(pgn) and has_10_moves(pgn)):
    send(game_no, buf)
