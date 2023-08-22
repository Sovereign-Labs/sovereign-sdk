
COMPOSE_FILE=docker-compose.celestia.yaml
MOUNT_FOLDER=keyring-test
NODE_1_KEY_FILE=bridge_1_key.txt

up:
	docker-compose --file $(COMPOSE_FILE) up -d


down:
	docker-compose --file "$(COMPOSE_FILE)" down
	rm -rf $(MOUNT_FOLDER)/*.txt
	rm -rf config_*.toml

restart: down up generate_configs

generate_configs:
	bash ./generate_configs.sh

logs:
	docker-compose --file "$(COMPOSE_FILE)" logs --follow