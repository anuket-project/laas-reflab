FROM alpine:latest

COPY influx_backup.sh /influx_backup.sh
COPY install_influx.sh /install_influx.sh

RUN chmod +x ./influx_backup.sh && chmod +x ./install_influx.sh && ./install_influx.sh

# Export environment variables for cron
RUN echo "export INFLUXDB_INIT_BUCKET=${INFLUXDB_INIT_BUCKET}" >> /etc/environment \
 && echo "export INFLUX_HOST=${INFLUX_HOST}" >> /etc/environment \
 && echo "export INFLUX_TOKEN=${INFLUX_TOKEN}" >> /etc/environment \
 && echo "export INFLUX_ORG=${INFLUX_ORG}" >> /etc/environment

RUN echo '0 2 * * * /influx_backup.sh > /var/log/influx_backup.log 2>&1' > /etc/crontabs/root

CMD ["crond", "-f", "-d", "8"]

