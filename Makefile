default: aoe2-tournament-bot.hash

test: aoe2-tournament-bot.hash
	docker run -v ${PWD}/service-account.json:/app/service-account.json:rw -v ${PWD}/token.json:/app/token.json:rw -e ADMIN_USER_ID=${ADMIN_USER_ID} -e DISCORD_TOKEN=${DISCORD_TOKEN} -e GCS_BUCKET="aoe2-tournament-replays" -e GOOGLE_APPLICATION_CREDENTIALS="service-account.json" --rm -it aoe2-tournament-bot:latest

publish: aoe2-tournament-bot.hash
	gcloud auth configure-docker europe-north1-docker.pkg.dev
	docker tag aoe2-tournament-bot:latest europe-north1-docker.pkg.dev/aoe2-tournaments/aoe2-tournament-bot/aoe2-tournament-bot:latest
	docker push europe-north1-docker.pkg.dev/aoe2-tournaments/aoe2-tournament-bot/aoe2-tournament-bot:latest

aoe2-tournament-bot.hash: Dockerfile requirements.txt bot.py
	docker buildx build --iidfile $@ --tag aoe2-tournament-bot .

token.json: credentials.json
	python3 create-token.py

.PHONY: default
