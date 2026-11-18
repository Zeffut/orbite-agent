# Native install, no container

Orbite is installed directly on the Host by one command and runs as a system service, rather than being shipped as a container image. The Agent's value comes from really living on a machine (its own account, its own Computer with a screen and a browser), and asking people to install and understand Docker first works against the goal of an install anyone can follow. Isolation comes from a dedicated system account and the Permission Level, not from a container boundary. Debian and Ubuntu come first; other systems follow.
