"""Check the runner with its file-backed credential; never print secrets."""
import base64
import os
from pathlib import Path
import urllib.request


def main():
    secret = Path(os.environ['K6_WEB_TOKEN_FILE']).read_text().strip()
    request = urllib.request.Request('http://127.0.0.1:5665/api/config')
    request.add_header('Authorization', 'Basic ' + base64.b64encode(('runner:' + secret).encode()).decode())
    with urllib.request.urlopen(request, timeout=3) as response:
        return 0 if response.status == 200 else 1


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except Exception:
        raise SystemExit(1)
