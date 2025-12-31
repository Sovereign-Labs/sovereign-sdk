.PHONY: db clean

DB_CONTAINER := sov-indexer-db

db:
	@echo "Initializing db"
	@if [ "$$(docker ps -a -q -f name=$(DB_CONTAINER))" ]; then \
		echo "Container already exists, starting it..."; \
		docker start $(DB_CONTAINER); \
	else \
		echo "Creating new container..."; \
		docker run --name $(DB_CONTAINER) \
			-e POSTGRES_PASSWORD=admin123 \
			-d \
			-p 5432:5432 \
			postgres; \
	fi
	@echo "Waiting for database to be ready..."
	@sleep 2
	@docker exec -i $(DB_CONTAINER) psql -U postgres < ./db/create_events_table.sql

clean:
	@echo "Cleaning up database container..."
	@docker stop $(DB_CONTAINER) 2>/dev/null || true
	@docker rm $(DB_CONTAINER) 2>/dev/null || true
	@echo "Cleanup complete"
