#!/usr/bin/env python3

import dataclasses
import io
import logging
import os
import re
import sys
from datetime import datetime, timezone
from typing import Optional, List, Any

import coloredlogs
import discord
import google.auth
from google.cloud import storage
from google.oauth2.credentials import Credentials
from googleapiclient import discovery

logger = logging.getLogger("AoE2TournamentBot")
coloredlogs.install(level="INFO")

ADMIN_USER_ID = int(os.environ["ADMIN_USER_ID"])
DISCORD_TOKEN = os.environ["DISCORD_TOKEN"]
GCS_BUCKET = os.environ["GCS_BUCKET"]
SHEET_ID = os.environ["SHEET_ID"]

SHEET_NAME = "AoE2 Results"
GOOGLE_SCOPES = [
    "https://www.googleapis.com/auth/spreadsheets",
]

EXTRA_RESULTS_CHANNELS = []


def optstr(x: Optional[Any]) -> str:
    return "" if x is None else str(x)


@dataclasses.dataclass
class ResultsEntry:
    message_link: str
    poster: str
    message_contents: str
    bracket: Optional[str] = None
    player1_id: Optional[int] = None
    player1_name: Optional[str] = None
    player1_score: Optional[int] = None
    player2_id: Optional[int] = None
    player2_name: Optional[str] = None
    player2_score: Optional[int] = None
    map_draft: Optional[str] = None
    civ_draft: Optional[str] = None
    replays_link: Optional[str] = None

    def get_row(self) -> List[str]:
        return [
            self.message_link,
            self.poster,
            optstr(self.bracket),
            optstr(self.player1_id),
            optstr(self.player1_name),
            optstr(self.player1_score),
            optstr(self.player2_id),
            optstr(self.player2_name),
            optstr(self.player2_score),
            optstr(self.map_draft),
            optstr(self.civ_draft),
            optstr(self.replays_link),
            self.message_contents,
        ]


def get_google_credentials():
    credentials, project_id = google.auth.default(scopes=GOOGLE_SCOPES)
    logger.info('Logged into GCP project "%s"', project_id)
    return credentials


def sheet_append_row(creds: Credentials, sheet_id: str, row: List[str]) -> bool:
    sheets = discovery.build("sheets", "v4", credentials=creds)
    values = [row]

    body = {"values": values}
    result = (
        sheets.spreadsheets()
        .values()
        .append(spreadsheetId=sheet_id, range="A1", valueInputOption="RAW", body=body)
        .execute()
    )
    logger.info(
        "Inserted %d new row(s) in spreadsheet", result["updates"]["updatedRows"]
    )

    return True


def validate_sheet_id(creds: Credentials, sheet_id: str) -> str:
    sheets = discovery.build("sheets", "v4", credentials=creds)
    sheet = sheets.spreadsheets().get(spreadsheetId=sheet_id).execute()
    logger.info('Writing to sheet titled "%s"', sheet["properties"]["title"])
    return sheet["spreadsheetId"]


def upload_gcs_file(filename: str, contents: bytes) -> None:
    storage_client = storage.Client()
    bucket = storage_client.bucket(GCS_BUCKET)
    blob = bucket.blob(filename)
    blob.upload_from_string(contents)


def parse_message_content(entry: ResultsEntry, content: str) -> ResultsEntry:
    # Find @User tags, hopefully from the "@User vs @User" part of the message
    discord_tag = re.compile(r"<@(\d+)>")
    if players_match := discord_tag.findall(content):
        if len(players_match) != 2:
            logger.info(
                "Found %d players in the message, expected 2", len(players_match)
            )
        else:
            entry.player1_id = int(players_match[0])
            entry.player2_id = int(players_match[1])

    content = discord_tag.sub("", content)

    # Try to match "maps: http://" or "map draft: http://..." with
    # optional whitespace everywhere and case ignored
    mapdraft = re.compile(r"maps?(?:\s+draft)?\s*:?\s*([^\s]+)", re.IGNORECASE)
    if mapdraft_match := mapdraft.search(content):
        entry.map_draft = mapdraft_match[1]
    content = mapdraft.sub("", content)

    # Try to match "civs: http://..." or "civ draft: http://..." with
    # optional whitespace everywhere and case ignored
    civdraft = re.compile(r"civs?(?:\s+draft)?\s*:?\s*([^\s]+)", re.IGNORECASE)
    if civdraft_match := civdraft.search(content):
        entry.civ_draft = civdraft_match[1]
    content = civdraft.sub("", content)

    # Try to match any two groups of digits separated by anything on the same line
    if score_match := re.search(
        r"^[^\d]*(\d{1,4})[^\d\v]+(\d{1,4})[^\d]*$", content, re.MULTILINE
    ):
        entry.player1_score = int(score_match[1])
        entry.player2_score = int(score_match[2])

    return entry


def is_results_channel(channel_name: str) -> bool:
    if channel_name.endswith("results"):
        return True
    if channel_name.startswith("results"):
        return True
    if channel_name in EXTRA_RESULTS_CHANNELS:
        return True
    return False


class AoE2TournamentBot(discord.Client):
    def __init__(self, google_credentials: Credentials, results_sheet_id: str):
        self.google_credentials = google_credentials
        self.results_sheet_id = results_sheet_id
        intents = discord.Intents.default()
        intents.message_content = True
        super().__init__(intents=intents)

    async def report_admin_error(self, error: str) -> None:
        admin = await self.fetch_user(ADMIN_USER_ID)
        await admin.send(f"AoE2 Tournament Bot error: {error}")

    async def on_ready(self) -> None:
        logger.info("Logged in We have logged in as %s", self.user)

    async def construct_results_entry(
        self, message: discord.Message
    ) -> Optional[ResultsEntry]:
        entry = ResultsEntry(
            message_link=message.to_reference().jump_url,
            poster=message.author.display_name,
            message_contents=message.content,
        )
        if isinstance(message.channel, discord.TextChannel) and (
            category := message.channel.category
        ):
            entry.bracket = category.name
        entry = parse_message_content(entry, message.content)

        if entry.player1_id is not None:
            entry.player1_name = (await self.fetch_user(entry.player1_id)).display_name
        if entry.player2_id is not None:
            entry.player2_name = (await self.fetch_user(entry.player2_id)).display_name
        download_links = []
        for idx, attachment in enumerate(message.attachments):
            attachment_io = io.BytesIO()
            await attachment.save(attachment_io)
            attachment_data = attachment_io.getvalue()
            filename = f"{attachment.id}_{attachment.filename}"
            logger.info(
                "Uploading attachment %d as %s with %d bytes of data",
                idx + 1,
                filename,
                len(attachment_data),
            )
            download_links.append(f"gcs://{GCS_BUCKET}/{filename}")
            upload_gcs_file(filename, attachment_data)

        entry.replays_link = "\n".join(download_links)
        return entry

    async def process_message(self, message: discord.Message) -> None:
        if message.author == self.user:
            return

        if not isinstance(message.channel, discord.TextChannel):
            return

        if not is_results_channel(message.channel.name):
            return

        logger.info(
            "Trying to process message with id %d as a results message", message.id
        )

        entry = await self.construct_results_entry(message)
        if not entry:
            logger.info(
                "The message with id %d does not seem to be a valid result", message.id
            )
            return

        logger.info("Creating results entry for message %d", message.id)

        now = datetime.now(tz=timezone.utc).replace(microsecond=0)
        row_to_add = [now.isoformat()] + entry.get_row()
        if not sheet_append_row(
            self.google_credentials, self.results_sheet_id, row_to_add
        ):
            await self.report_admin_error(
                "Failed to append results row for message %d. Please check logs",
                message.id,
            )

    async def on_message(self, message: discord.Message) -> None:
        logger.debug("Processing new message with ID %d", message.id)
        return await self.process_message(message)

    async def on_message_edit(
        self, before: discord.Message, after: discord.Message
    ) -> None:
        logger.debug("Processing updated message with ID %d", after.id)
        return await self.process_message(after)


def main() -> int:
    logger.info('Replays will be saved to bucket "%s"', GCS_BUCKET)
    logger.info("Errors will be sent to user %d", ADMIN_USER_ID)

    google_credentials = get_google_credentials()
    if not google_credentials:
        return 1

    logger.info("Google API credentials valid")

    results_sheet_id = validate_sheet_id(google_credentials, SHEET_ID)
    if not results_sheet_id:
        return 1

    logger.info("Results sheet set up")

    client = AoE2TournamentBot(google_credentials, results_sheet_id)
    client.run(DISCORD_TOKEN, log_handler=None)
    logger.info("Shutting down...")
    return 0


if __name__ == "__main__":
    sys.exit(main())
