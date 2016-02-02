#!/usr/bin/env python

import chess
import chess.pgn
import requests
import random
import sys
import itertools

f = open(sys.argv[1])

c = itertools.count(1)

while True:
    game = chess.pgn.read_game(f)
    if game is None:
        break

    res = requests.put("http://localhost:9000/", data=str(game))
    print(next(c), res, res.text)
    if res.status_code != 200:
        print(game)
