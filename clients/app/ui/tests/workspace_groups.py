"""Widgets and enabled packages open their tabs beside the chat. Checks that
exercise one group of tabs merge the board first and show the chat."""


def merge_groups(js, wait_for):
    if js("return document.querySelectorAll('.workspace-group:not([hidden])').length===2"):
        js("document.querySelector('.topbar [data-workspace-split]').click()")
        wait_for(lambda: js("return document.querySelectorAll('.workspace-group:not([hidden])').length===1"), 'Groups did not merge')
    js("document.querySelector('[data-tab-id=\"client:chat\"] [role=tab]')?.click()")
    wait_for(lambda: js("return !!document.querySelector('.session-workspace')?.getBoundingClientRect().width"), 'Chat is not visible after merging groups')
