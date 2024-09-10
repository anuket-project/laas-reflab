FROM grafana/grafana:11.1.0

ENV GF_SECURITY_ADMIN_USER=${GRAFANA_ADMIN_USER}
ENV GF_SECURITY_ADMIN_PASSWORD=${GRAFANA_ADMIN_PASSWORD}
ENV INFLUX_TOKEN=${INFLUX_TOKEN}
ENV INFLUX_HOST=${INFLUX_HOST}
ENV INFLUX_USER=${INFLUX_USER}
ENV INFLUX_BUCKET=${INFLUXDB_INIT_BUCKET}
ENV LDAP_BIND_PASSWORD=${LDAP_BIND_PASSWORD}
ENV LDAP_HOST=${LDAP_HOST}
ENV LDAP_BIND_DN=${LDAP_BIND_DN}

COPY ./datasources /etc/grafana/provisioning/datasources
COPY ./dashboards /etc/grafana/provisioning/dashboards
COPY ./grafana.ini /etc/grafana/grafana.ini
COPY ./ldap.toml /etc/grafana/ldap.toml
COPY ./certificates/ca.crt /etc/ssl/certs/ca.crt

USER $USERID

EXPOSE 3000

CMD ["grafana-server", "--homepat  h=/usr/share/grafana", "--config=/etc/grafana/grafana.ini"]

