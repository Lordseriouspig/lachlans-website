FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        tor \
        gosu \
    && rm -rf /var/lib/apt/lists/*

COPY torrc /etc/tor/torrc
COPY entrypoint.sh /entrypoint.sh

RUN chmod 755 /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]
