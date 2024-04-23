#!/bin/sh

set -e

./influx backup --bucket ${INFLUXDB_INIT_BUCKET} --host ${INFLUX_HOST} --token ${INFLUX_TOKEN} --org ${INFLUX_ORG} /backups
