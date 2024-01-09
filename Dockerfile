FROM docker.io/rustlang/rust:nightly

WORKDIR /usr/src/liblaas

RUN apt-get clean && apt-get update -y && apt-get upgrade -y && apt-get install -y python3-dev postgresql-client ipmitool iputils-ping

COPY ./src .
RUN cargo install --path .
COPY ./templates ./templates
CMD ["laas-reflab", "--server"]
