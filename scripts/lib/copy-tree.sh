# Sourced by the image builders (scripts/wsl/build-operator.sh, distro/vm/build-vm.sh).
#
# copy_tree SRC DST: a tree from the checkout into an image, with modes of its own rather
# than the source's. A checkout read through a Windows drive reports 0777 for everything,
# and `cp -a SRC/. /` would make / and /usr writable by anyone. Directories that exist keep
# their mode; new ones are 0755; scripts (#!) and programs (ELF) 0755; the rest 0644.
copy_tree() {
  local src=$1 dst=$2 path magic
  while IFS= read -r -d '' path; do
    [[ -d $dst/$path ]] || install -d -m 0755 "$dst/$path"
  done < <(cd "$src" && find . -mindepth 1 -type d -print0)
  while IFS= read -r -d '' path; do
    if [[ -L $src/$path ]]; then cp -P --remove-destination "$src/$path" "$dst/$path"; continue; fi
    magic=$(head -c 4 "$src/$path" | od -An -tx1 | tr -d ' \n')
    if [[ $magic == 2321* || $magic == 7f454c46 ]]; then install -m 0755 "$src/$path" "$dst/$path"
    else install -m 0644 "$src/$path" "$dst/$path"; fi
  done < <(cd "$src" && find . \( -type f -o -type l \) -print0)
}

# refuse_world_writable ROOT: fails if anything in ROOT but a sticky directory (/tmp) is
# writable by anyone.
refuse_world_writable() {
  local found
  found=$(find "$1" -xdev -perm -0002 \( -type f -o -type d ! -perm -1000 \) -print -quit)
  [[ -z $found ]] || { echo "${found#"$1"} is writable by anyone" >&2; return 65; }
}
