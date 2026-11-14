FROM docker:27.5.1-cli AS docker-client
FROM python:3.11-slim-bookworm
RUN apt-get update && apt-get install -y --no-install-recommends default-mysql-client tzdata && rm -rf /var/lib/apt/lists/*
COPY --from=docker-client /usr/local/bin/docker /usr/local/bin/docker
COPY healthcheck.py /opt/k6-web-healthcheck.py
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1
EXPOSE 5665
STOPSIGNAL SIGINT
HEALTHCHECK --interval=5s --timeout=5s --start-period=10s --retries=6 CMD ["python3", "/opt/k6-web-healthcheck.py"]
ENTRYPOINT ["python3"]
