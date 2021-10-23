import requests
import chess
import chess.pgn
import sys


def main(pgn):
    session = requests.session()

    while True:
        game = chess.pgn.read_game(pgn)
        if game is None:
            break

        res = session.put("http://localhost:9000/import/master", json={
            "id": game.headers["LichessId"],
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
        })

        if res.status_code != 200:
            print(res.text)

        break


def winner(game):
    if game.headers["Result"] == "1-0":
        return "white"
    elif game.headers["Result"] == "0-1":
        return "black"
    elif game.headers["Result"] == "1/2-1/2":
        return None
    else:
        assert False, "invalid result"


if __name__ == "__main__":
    main(open(sys.argv[1]))

