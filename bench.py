import chess.pgn
import random
import requests
import sys
import time

session = requests.session()

stats = []

with open(sys.argv[1]) as pgn:
    while len(stats) < 10000:
        board = chess.pgn.read_game(pgn, Visitor=chess.pgn.BoardBuilder)
        if board is None:
            break

        random_ply = random.randint(0, len(board.move_stack))

        while len(board.move_stack) > random_ply:
            board.pop()

        start_time = time.perf_counter()
        res = session.get("http://localhost:9002/lichess", params={"fen": board.epd()})
        res.raise_for_status()
        elapsed = time.perf_counter() - start_time

        stats.append(elapsed)

print(sum(stats) / len(stats))
