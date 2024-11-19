import bot

TEST_MESSAGE1 = """
<@698259349415657522> vs. <@810249574173245501>  Recruit SF 
Civs: https://aoe2cm.net/draft/SfNXP 
Map: https://aoe2cm.net/draft/zQKpk
"""


def test_message1():
    entry = bot.ResultsEntry("", "", "")
    entry = bot.parse_message_content(entry, TEST_MESSAGE1)
    assert entry.player1_id == 698259349415657522, entry.player1_id
    assert entry.player2_id == 810249574173245501, entry.player2_id
    assert entry.civ_draft == "https://aoe2cm.net/draft/SfNXP", entry.civ_draft
    assert entry.map_draft == "https://aoe2cm.net/draft/zQKpk", entry.map_draft


TEST_MESSAGE2 = """
<@698259349415657522> 3-0 <@810249574173245501>  Recruit SF 
Civs: https://aoe2cm.net/draft/SfNXP 
Map: https://aoe2cm.net/draft/zQKpk
"""


def test_message2():
    entry = bot.ResultsEntry("", "", "")
    entry = bot.parse_message_content(entry, TEST_MESSAGE2)
    assert entry.player1_id == 698259349415657522, entry.player1_id
    assert entry.player2_id == 810249574173245501, entry.player2_id
    assert entry.civ_draft == "https://aoe2cm.net/draft/SfNXP", entry.civ_draft
    assert entry.map_draft == "https://aoe2cm.net/draft/zQKpk", entry.map_draft
    assert entry.player1_score == 3, entry.player1_score
    assert entry.player2_score == 0, entry.player2_score


TEST_MESSAGE3 = """
<@359062701831618560> ||0:3|| <@271375929702350849> 
General SF
Map draft: https://aoe2cm.net/draft/TlCgx
Civ draft: https://aoe2cm.net/draft/vlrcX
"""


def test_message3():
    entry = bot.ResultsEntry("", "", "")
    entry = bot.parse_message_content(entry, TEST_MESSAGE3)
    assert entry.player1_id == 359062701831618560, entry.player1_id
    assert entry.player2_id == 271375929702350849, entry.player2_id
    assert entry.civ_draft == "https://aoe2cm.net/draft/vlrcX", entry.civ_draft
    assert entry.map_draft == "https://aoe2cm.net/draft/TlCgx", entry.map_draft
    assert entry.player1_score == 0, entry.player1_score
    assert entry.player2_score == 3, entry.player2_score
