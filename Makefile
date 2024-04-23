build:
	docker compose -f docker-compose.yml -f docker-compose.metrics.yml build

up:
	docker compose -f docker-compose.yml -f docker-compose.dev.yml -f docker-compose.metrics.yml up 

deploy:
	@docker compose -f docker-compose.yml -f docker-compose.metrics.yml -f docker-compose.prod.yml up -d
	@echo -e "\e[94mStarting laas-reflab...\e[0m"
	@echo -e "\e[94mConnect to CLI with \e[0m'\e[92mmake cli\e[0m'"

cli:
	@docker exec -it $${PWD##*/}-liblaas-1 /bin/bash -c "laas-reflab --cli"

stop:
	@docker compose -f docker-compose.yml -f docker-compose.metrics.yml stop

edit-config:
	vim /var/lib/docker/volumes/$${PWD##*/}_config_data/_data/config.yaml

db-shell:
	docker exec -it --user postgres $${PWD##*/}-db-1 psql -d liblaas
