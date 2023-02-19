import chess.pgn
import sys

pgn = open(sys.argv[1])

def rating_group(avg):
    if avg >= 2500:
        return 2500
    elif avg >= 2200:
        return 2200
    elif avg >= 2000:
        return 2000
    elif avg >= 1800:
        return 1800
    elif avg >= 1600:
        return 1600
    elif avg >= 1501:
        return 1501
    else:
        return 0

def speed_group(time_control):
    if time_control == "-":
        return "correspondence"
    time, inc = time_control.split("+")
    estimate = int(time) + 40 * int(inc)
    if estimate < 30:
        return "ultraBullet"
    elif estimate < 180:
        return "bullet"
    elif estimate < 480:
        return "blitz"
    elif estimate < 1500:
        return "rapid"
    else:
        return "classical"

def elo(s):
    if s == "?":
        return 0
    else:
        return int(s)

counts = {}
total = 0

try:
    while True:
        headers = chess.pgn.read_headers(pgn)
        if headers is None:
            break

        avg_rating = (elo(headers["WhiteElo"]) + elo(headers["BlackElo"])) // 2
        group = rating_group(avg_rating)
        speed = speed_group(headers["TimeControl"])
        counts[(group, speed)] = counts.get((group, speed), 0) + 1
        total += 1
except KeyboardInterrupt:
    pass

for group in [0, 1501, 1600, 1800, 2000, 2200, 2500]:
    print("\t".join(str(counts.get((group, speed), 0) / total) for speed in ["ultraBullet", "bullet", "blitz", "rapid", "classical", "correspondence"]))
