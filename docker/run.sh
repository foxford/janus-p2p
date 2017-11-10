#!/bin/bash

PROJECT='janus-p2p'
PROJECT_DIR="/opt/sandbox/${PROJECT}"
DOCKER_CONTAINER_NAME="sandbox/${PROJECT}"
DOCKER_CONTAINER_COMMAND=${DOCKER_CONTAINER_COMMAND:-'/bin/bash'}
DOCKER_RUN_OPTIONS=${DOCKER_RUN_OPTIONS:-'-ti --rm'}

read -r DOCKER_RUN_COMMAND <<-EOF
    source ~/.profile \
    && ln -s "${PROJECT_DIR}/target/debug/libjanus_p2p.so" /opt/janus/lib/janus/plugins/libjanus_p2p.so \
    && service nginx start \
    && ln -s /opt/janus/bin/janus /usr/local/bin/janus
EOF

docker volume create janus-p2p-cargo
docker build -t ${DOCKER_CONTAINER_NAME} docker/
docker run ${DOCKER_RUN_OPTIONS} \
    -v $(pwd):${PROJECT_DIR} \
    -v janus-p2p-cargo:/root/.cargo \
    -p 8443:8443 \
    -p 8089:8089 \
    -p 5002:5002/udp \
    -p 5004:5004/udp \
    ${DOCKER_CONTAINER_NAME} \
    /bin/bash -c "set -x && cd ${PROJECT_DIR} && ${DOCKER_RUN_COMMAND} && set +x && ${DOCKER_CONTAINER_COMMAND}"
