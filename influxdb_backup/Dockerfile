FROM alpine:latest

COPY influx_backup.sh /influx_backup.sh
COPY install_influx.sh /install_influx.sh
RUN chmod +x ./influx_backup.sh && chmod +x ./install_influx.sh && ./install_influx.sh && echo '0 2 * * * /influx_backup.sh' > /etc/crontabs/root

CMD ["crond", "-f", "-d", "8"]
