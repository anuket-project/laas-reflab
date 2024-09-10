# make sure to have the API key and port env set
# must run on the same machine as the docker service running grafana
docker run --add-host host.docker.internal:host-gateway --rm trivago/hamara ./hamara export --host=host.docker.internal:${GRAFANA_PORT} --key=$GRAFANA_API_KEY >datasources.yaml
cat datasources.yaml
