#!/usr/bin/env python3

import sys
import requests

from kyotocabinet import *


if len(sys.argv) != 2:
    sys.exit("Usage: python3 migrate-master.py <master-pgn.kct>")

db = DB()
if not db.open(sys.argv[1], DB.OREADER):
    sys.exit(str(db.error()))


num_games = db.count()
n = 0

cur = db.cursor()
cur.jump()
while True:
    # Get next game id and pgn.
    pair = cur.get_str(True)
    if not pair:
        break

    game_id, pgn = pair

    # Skip games with unknown result.
    if "[Result \"*\"]" in pgn:
        print("Skipping result * in", game_id)
        continue

    # Keep the ID.
    buf = "[LichessID \"%s\"]\n%s" % (game_id, pgn)

    res = requests.put("http://localhost:9000/import/master", data=buf.rstrip())
    if res.status_code != 200:
        print(buf)
        print("---")
        print(res.status_code, res.text)
        sys.exit(1)

    # Progress report.
    n += 1
    if n % 10 == 0:
        print("%10d / %d" % (n, num_games))


if not db.close():
    sys.exit(str(db.error()))
