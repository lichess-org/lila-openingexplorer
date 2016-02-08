#!/usr/bin/env python

RATING_GROUPS = 5
SPEED_GROUPS = 3
GROUPS = RATING_GROUPS * SPEED_GROUPS

def pack_format_1(n):
    return 1 + (n * 8)

def master_pack_format_2(n):
    return 1 + (1 + 1 + 1 + 6) + 8 * min(n, 5)

def lichess_pack_format_2(n):
    return 1 + (1 + 1 + 1 + 6) * GROUPS + 8 * min(n, 5 * GROUPS)

print("games", "f1", "master2", "lichess2", sep="\t")
for n in range(1, 100):
    print(n, pack_format_1(n), master_pack_format_2(n), lichess_pack_format_2(n), sep="\t")
