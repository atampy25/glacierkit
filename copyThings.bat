@echo off
robocopy static build/_app/immutable/assets 32px.png 32px.png
robocopy static build/_app/immutable/assets throbber.gif throbber.gif
exit 0