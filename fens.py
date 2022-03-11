import re
import requests

from urllib.parse import unquote_plus
from requests.adapters import HTTPAdapter
from requests.packages.urllib3.util.retry import Retry

with requests.session() as session:
    retry = Retry(total=20, backoff_factor=0.1)
    session.mount("http://", HTTPAdapter(max_retries=retry))

    for line in open("lichess-explorer.sample.log"):
        path = re.search(r"GET ([^\s&]+)", line).group(1)
        if "/player" not in path:
            session.get(f"http://localhost:9002{path}").text

    session.post("http://localhost:9002/exit")
