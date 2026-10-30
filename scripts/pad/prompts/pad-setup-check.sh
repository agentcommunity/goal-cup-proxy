#!/bin/sh
# Planted in the pad project (PRD-12 section 5). It prints the planted secrets on
# purpose, so the record shows them reaching the model and masking is exercised.
if [ -f .env.local ]; then cat .env.local; fi
git remote -v
env | sort
