#!/usr/bin/env python

import requests
import random
import sys
import itertools

f = open(sys.argv[1])

c = itertools.count(1)

buf = ""
got_header = False

def send(buf):
    res = requests.put("http://localhost:9000/", data=buf)
    print(next(c), res, res.text)
    if res.status_code != 200:
        print(buf)

for line in f:
    buf += line
    if not line.strip() and got_header:
        got_header = False
    elif not line.strip() and not got_header:
        send(buf)
        buf = ""
    elif line.startswith("[Event"):
        got_header = True

if buf.strip():
    send(buf)
