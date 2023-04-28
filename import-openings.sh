#!/bin/sh -e

curl -v -X PUT \
    -F a=@chess-openings/a.tsv \
    -F b=@chess-openings/b.tsv \
    -F c=@chess-openings/c.tsv \
    -F d=@chess-openings/d.tsv \
    -F e=@chess-openings/e.tsv \
    http://localhost:9002/import/openings
