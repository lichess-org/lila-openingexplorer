import base64
import chess
import chess.pgn
import hashlib
import json
import requests
import sys


def main(pgn):
    session = requests.session()

    while True:
        game = chess.pgn.read_game(pgn)
        if game is None:
            break

        obj = {
            "event": game.headers["Event"],
            "site": game.headers["Site"],
            "date": game.headers["Date"],
            "round": game.headers["Round"],
            "white": {
                "name": game.headers["White"],
                "rating": int(game.headers["WhiteElo"]),
            },
            "black": {
                "name": game.headers["Black"],
                "rating": int(game.headers["BlackElo"]),
            },
            "winner": winner(game),
            "moves": " ".join(m.uci() for m in game.end().board().move_stack)
        }

        obj["LichessId"] = game.headers.get("LichessId") or deterministic_id(obj)

        res = session.put("http://localhost:9002/import/masters", json=obj)

        if res.status_code != 200:
            print(res.text)
        else:
            print(obj["LichessId"])


def winner(game):
    if game.headers["Result"] == "1-0":
        return "white"
    elif game.headers["Result"] == "0-1":
        return "black"
    elif game.headers["Result"] == "1/2-1/2":
        return None
    else:
        assert False, "invalid result"


def deterministic_id(obj):
    digest = hashlib.sha256()
    digest.update(json.dumps(obj, sort_keys=True).encode("utf-8"))
    return base64.b64encode(digest.digest(), b"ab")[0:8].decode("utf-8")


if __name__ == "__main__":
    main(open(sys.argv[1]))

