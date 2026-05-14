default: aoe2-tournament-bot.hash

test: aoe2-tournament-bot.hash
	docker run \
		-v ${PWD}/config.toml:/app/config.toml:ro \
		-v ${PWD}/service-account.json:/app/service-account.json:ro \
		-e GOOGLE_APPLICATION_CREDENTIALS=/app/service-account.json \
		--rm -it aoe2-tournament-bot:latest

publish: aoe2-tournament-bot.hash
	gcloud auth configure-docker europe-north1-docker.pkg.dev
	docker tag aoe2-tournament-bot:latest europe-north1-docker.pkg.dev/aoe2-tournaments/aoe2-tournament-bot/aoe2-tournament-bot:latest
	docker push europe-north1-docker.pkg.dev/aoe2-tournaments/aoe2-tournament-bot/aoe2-tournament-bot:latest

aoe2-tournament-bot.hash: Dockerfile Cargo.toml Cargo.lock $(shell find src -type f)
	docker buildx build --iidfile $@ --tag aoe2-tournament-bot .

.PHONY: default test publish
