FROM python@sha256:5e2bdb1b0d542d5039cffb8181c65930093949194a65c365dfb1b0fe13a4fa5b

LABEL org.opencontainers.image.source="https://github.com/Miosa-osa/SOMA"
LABEL org.opencontainers.image.description="Gymnasium FrozenLake reinforcement-learning workload for SOMA"
LABEL org.opencontainers.image.licenses="Apache-2.0"

WORKDIR /opt/soma-rl
COPY requirements.txt .
RUN pip install --no-cache-dir --only-binary=:all: -r requirements.txt
COPY train.py rollouts.py .

ENTRYPOINT ["/usr/local/bin/python3", "/opt/soma-rl/train.py"]
