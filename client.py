#!/usr/bin/env python

import chess
import chess.pgn
import requests
import random

game = chess.pgn.Game()
node = game

board = game.board()

while not board.is_game_over(claim_draw=True):
    move = random.choice(list(board.legal_moves))
    node = node.add_variation(move)
    board.push(move)

game.headers["Result"] = board.result(claim_draw=True)

print(game)

print()

res = requests.put("http://localhost:9000/", data=str(game))
print(res)
print(res.text)
print(res)
