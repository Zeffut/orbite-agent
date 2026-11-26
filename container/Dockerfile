# An Agent in a container of its own (ADR 0022): the program, and the
# computer it uses, a screen, what moves the pointer and types, a browser
# and fonts. Everything the Agent keeps is in /var/lib/orbite, which is a
# volume: the container can be thrown away and made again, the Agent stays.
#
# Built with the program beside this file, as `orbite`, for the machine the
# image is for.
FROM debian:13-slim

# The Agent's computer, as the installer sets one up on a Debian; then
# what a person working at a computer expects to find there.
RUN apt-get update -qq \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
        xvfb xdotool firefox-esr \
        fonts-liberation fonts-noto-core fonts-noto-cjk fonts-noto-color-emoji \
        ca-certificates curl git python3 jq unzip zip file procps less \
    && rm -rf /var/lib/apt/lists/*

# The Agent is not root in its own container either.
RUN useradd --system --create-home --home-dir /var/lib/orbite --shell /usr/sbin/nologin orbite \
    && mkdir -p /usr/local/share/orbite \
    # Its computer is its own: there is no desktop of the machine's here.
    && echo own > /usr/local/share/orbite/computer

COPY orbite /usr/local/bin/orbite
RUN chmod 755 /usr/local/bin/orbite

USER orbite
ENV ORBITE_HOME=/var/lib/orbite
WORKDIR /var/lib/orbite
VOLUME /var/lib/orbite
# Where the server in front reaches this Agent: on every address of the
# container, which only that server is let near. What travels there is
# encrypted by the server, on its way to the Owner's devices.
ENV ORBITE_INTERFACE=0.0.0.0:8377
ENV ORBITE_INTERFACE_IN_THE_OPEN=1
EXPOSE 8377

CMD ["orbite", "run"]
