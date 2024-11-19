#!/usr/bin/env python3

import dataclasses
import io
import logging
import re
import os
import sys
from pathlib import Path
from typing import Optional, List, Any

import coloredlogs
import discord
from google.auth.transport.requests import Request
from google.cloud import storage
from google.oauth2.credentials import Credentials
from googleapiclient import discovery

logger = logging.getLogger("AoE2TournamentBot")
coloredlogs.install(level="INFO")

ADMIN_USER_ID = int(os.environ["ADMIN_USER_ID"])
DISCORD_TOKEN = os.environ["DISCORD_TOKEN"]
GCS_BUCKET = os.environ["GCS_BUCKET"]

SHEET_NAME = "AoE2 Results"
GOOGLE_SCOPES = ["https://www.googleapis.com/auth/drive.file"]

RESULTS_CHANNELS = ["results"]


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


def get_google_credentials() -> Optional[Credentials]:
    token_path = Path("token.json")
    try:
        creds = Credentials.from_authorized_user_file(str(token_path), GOOGLE_SCOPES)
    except:
        logger.error(
            "No token.json found. Please set up Google API credentials manually first"
        )
        return None

    return google_ensure_credentials(creds)


def google_ensure_credentials(creds: Credentials) -> Optional[Credentials]:
    if creds.valid:
        return creds

    if creds.expired and creds.refresh_token:
        try:
            creds.refresh(Request())
        except Exception as e:
            logger.error("Failed to refresh Google API credentials, error: %s", str(e))
            return None
    else:
        logger.error("Google API credentials are not valid")
        return None

    with open("token.json", "w") as token:
        token.write(creds.to_json())

    return creds


def get_replay_sheet_id(creds: Credentials) -> Optional[str]:
    valid_creds = google_ensure_credentials(creds)
    if not valid_creds:
        return None

    files = discovery.build("drive", "v3", credentials=valid_creds)
    filelist = files.files().list(q=f'name = "{SHEET_NAME}"').execute()

    if len(filelist["files"]) > 1:
        logger.error('Multiple files named "{SHEET_NAME}"')
        return None
    elif len(filelist["files"]) == 1:
        results_sheet = filelist["files"][0]
        logger.info("Found results spreadsheet with id %s", results_sheet["id"])
        existing_sheet_id: str = results_sheet["id"]
        return existing_sheet_id
    elif len(filelist["files"]) == 0:
        sheets = discovery.build("sheets", "v4", credentials=valid_creds)
        spreadsheet = {"properties": {"title": SHEET_NAME}}
        results_sheet = (
            sheets.spreadsheets()
            .create(body=spreadsheet, fields="spreadsheetId")
            .execute()
        )
        logger.info(
            "Created new results spreadsheet with id %s", results_sheet["spreadsheetId"]
        )

        created_sheet_id: str = results_sheet["spreadsheetId"]
        sheet_append_row(
            valid_creds,
            created_sheet_id,
            [
                "Message link",
                "Poster Display Name",
                "Bracket",
                "Player 1 ID",
                "Player 1 Display Name",
                "Player 1 Score",
                "Player 2 Display Name",
                "Player 2 ID",
                "Player 2 Score",
                "Map Draft",
                "Civ Draft",
                "Replay links",
                "Message contents",
            ],
        )
        return created_sheet_id
    else:
        logger.error("File list has negative length")
        return None


def sheet_append_row(creds: Credentials, sheet_id: str, row: List[str]) -> bool:
    valid_creds = google_ensure_credentials(creds)
    if not valid_creds:
        return False
    sheets = discovery.build("sheets", "v4", credentials=valid_creds)
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


def upload_gcs_file(filename: str, contents: bytes) -> None:
    storage_client = storage.Client()
    bucket = storage_client.bucket(GCS_BUCKET)
    blob = bucket.blob(filename)
    blob.upload_from_string(contents)


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

        # Try to match @User ||3-2|| @User with the spoiler markers and all whitespace being optional
        if score_match := re.search(
            r"<@(\d+)>\s*(?:\|\|)?\s*(\d+)\s*-\s*(\d+)\s*(?:\|\|)?\s*<@(\d+)>",
            message.content,
        ):
            entry.player1_id = int(score_match[1])
            entry.player1_name = (await self.fetch_user(entry.player1_id)).display_name
            entry.player1_score = int(score_match[2])
            entry.player2_id = int(score_match[4])
            entry.player2_name = (await self.fetch_user(entry.player2_id)).display_name
            entry.player2_score = int(score_match[3])

        # Try to match "map draft: http://..." with optional whitespace everywhere and case ignored
        if mapdraft_match := re.search(
            r"map\s+draft\s*:?\s*([^\s]+)", message.content, re.IGNORECASE
        ):
            entry.map_draft = mapdraft_match[1]

        # Try to match "civ draft: http://..." with optional whitespace everywhere and case ignored
        if civdraft_match := re.search(
            r"civ\s+draft\s*:?\s*([^\s]+)", message.content, re.IGNORECASE
        ):
            entry.civ_draft = civdraft_match[1]

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

    async def on_message(self, message: discord.Message) -> None:
        if message.author == self.user:
            return

        if not isinstance(message.channel, discord.TextChannel):
            return

        if message.channel.name not in RESULTS_CHANNELS:
            return

        entry = await self.construct_results_entry(message)
        if not entry:
            logger.info("The message does not seem to be a valid result")
            return

        logger.info("Creating results entry for message %d", message.id)

        if not sheet_append_row(
            self.google_credentials, self.results_sheet_id, entry.get_row()
        ):
            await self.report_admin_error(
                "Failed to append results row. Please check logs"
            )


def main() -> int:
    logger.info('Replays will be saved to bucket "%s"', GCS_BUCKET)
    logger.info('Errors will be sent to user %d', ADMIN_USER_ID)

    google_credentials = get_google_credentials()
    if not google_credentials:
        return 1
    
    logger.info('Google API credentials valid')

    results_sheet_id = get_replay_sheet_id(google_credentials)
    if not results_sheet_id:
        return 1
    
    logger.info('Results sheet set up')


    client = AoE2TournamentBot(google_credentials, results_sheet_id)
    client.run(DISCORD_TOKEN, log_handler=None)
    return 0


if __name__ == "__main__":
    sys.exit(main())
