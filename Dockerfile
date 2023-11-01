FROM docker.io/rustlang/rust:nightly

WORKDIR /usr/src/liblaas

RUN apt-get clean && apt-get update -y && apt-get upgrade -y && apt-get install -y python3-dev postgresql-client ipmitool iputils-ping

COPY ./src .
COPY ./templates ./templates
RUN cargo install --path .
CMD ["laas-reflab", "--server"]
