#!/usr/bin/env python3

import sys
import requests
import csv


def main(file):
    session = requests.session()

    reader = csv.reader(file, delimiter="\t")
    next(reader, None)

    for row in reader:
        eco, name, pgn, _, fen = row

        req = session.get("http://localhost:9001/masters", params={"fen": fen})
        req.raise_for_status()
        masters = req.json()

        req = session.get("http://localhost:9001/lichess", params={"fen": fen})
        req.raise_for_status()
        lichess = req.json()

        total_masters = masters["white"] + masters["black"] + masters["draws"]
        total_lichess = lichess["white"] + lichess["black"] + lichess["draws"]

        if total_masters + total_lichess < 5:
            print(eco, name, pgn, total_masters, total_lichess)


if __name__ == "__main__":
    for arg in sys.argv[1:]:
        with open(arg) as file:
            main(file)
