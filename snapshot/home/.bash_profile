# .bash_profile

# Get the aliases and functions
if [ -f ~/.bashrc ]; then
    . ~/.bashrc
fi

# User specific environment and startup programs


# Added by Antigravity CLI installer
export PATH="/home/aqibj/.local/bin:$PATH"

# Mac-style cursor in the COSMIC session only (GNOME uses its own setting)
if [ "$XDG_CURRENT_DESKTOP" = "COSMIC" ]; then
    export XCURSOR_THEME=WhiteSur-cursors XCURSOR_SIZE=24
fi
