Test that rename detection works
  $ . "$TESTDIR/hggit/testutil"

  $ cat >> $HGRCPATH <<EOF
  > [diff]
  > git = True
  > [git]
  > similarity = 50
  > EOF

  $ git init -q gitrepo
  $ cd gitrepo
  $ for i in 1 2 3 4 5 6 7 8 9 10; do echo $i >> alpha; done
  $ git add alpha
  $ fn_git_commit -malpha

Rename a file
  $ git mv alpha beta
  $ echo 11 >> beta
  $ git add beta
  $ fn_git_commit -mbeta

Copy a file
  $ cp beta gamma
  $ echo 12 >> beta
  $ echo 13 >> gamma
  $ git add beta gamma
  $ fn_git_commit -mgamma

Add a submodule (gitlink) and move it to a different spot:
  $ cd ..
  $ git init -q gitsubmodule
  $ cd gitsubmodule
  $ touch subalpha
  $ git add subalpha
  $ fn_git_commit -msubalpha
  $ cd ../gitrepo

  $ git submodule add ../gitsubmodule
  Cloning into '$TESTTMP/gitrepo/gitsubmodule'...
  done.
  $ fn_git_commit -m 'add submodule'
  $ sed -e 's/path = gitsubmodule/path = gitsubmodule2/' .gitmodules > .gitmodules-new
  $ mv .gitmodules-new .gitmodules
  $ mv gitsubmodule gitsubmodule2

Previous versions of git did not produce any output but 2.14 changed the output
to warn the user about submodules

  $ git add .gitmodules gitsubmodule2 2>/dev/null
  $ git rm --cached gitsubmodule
  rm 'gitsubmodule'
  $ fn_git_commit -m 'move submodule'

Rename a file elsewhere and replace it with a symlink:

  $ git mv beta beta-new
  $ ln -s beta-new beta
  $ git add beta
  $ fn_git_commit -m 'beta renamed'

Rename the file back:

  $ git rm beta
  rm 'beta'
  $ git mv beta-new beta
  $ fn_git_commit -m 'beta renamed back'

Rename a file elsewhere and replace it with a submodule:

  $ git mv gamma gamma-new
  $ git submodule add ../gitsubmodule gamma 2>&1
  Cloning into '$TESTTMP/gitrepo/gamma'...
  done.
  $ fn_git_commit -m 'rename and add submodule'

Remove the submodule and rename the file back:

  $ grep 'submodule "gitsubmodule"' -A2 .gitmodules > .gitmodules-new
  $ mv .gitmodules-new .gitmodules
  $ git add .gitmodules
  $ git rm --cached gamma
  rm 'gamma'
  $ rm -rf gamma
  $ git mv gamma-new gamma
  $ fn_git_commit -m 'remove submodule and rename back'

  $ git checkout -f -b not-master 2>&1
  Switched to a new branch 'not-master'

  $ cd ..
  $ hg clone -q gitrepo hgrepo
  $ cd hgrepo
  $ hg book master -q
  $ hg log -p --graph --template "{rev} {node} {desc|firstline}\n{join(extras, ' ')}\n\n"
  @  8 2e3567dd7aeb6c566bfe675119b2cc008d042c26 remove submodule and rename back
  |  branch=default convert_revision=50d116676a308b7c22935137d944e725d2296f2a hg-git-rename-source=git
  |
  |  diff --git a/.gitmodules b/.gitmodules
  |  --- a/.gitmodules
  |  +++ b/.gitmodules
  |  @@ -1,6 +1,3 @@
  |   [submodule "gitsubmodule"]
  |   	path = gitsubmodule2
  |   	url = ../gitsubmodule
  |  -[submodule "gamma"]
  |  -	path = gamma
  |  -	url = ../gitsubmodule
  |  diff --git a/.hgsub b/.hgsub
  |  --- a/.hgsub
  |  +++ b/.hgsub
  |  @@ -1,2 +1,1 @@
  |   gitsubmodule2 = [git]../gitsubmodule
  |  -gamma = [git]../gitsubmodule
  |  diff --git a/.hgsubstate b/.hgsubstate
  |  --- a/.hgsubstate
  |  +++ b/.hgsubstate
  |  @@ -1,2 +1,1 @@
  |  -5944b31ff85b415573d1a43eb942e2dea30ab8be gamma
  |   5944b31ff85b415573d1a43eb942e2dea30ab8be gitsubmodule2
  |  diff --git a/gamma-new b/gamma
  |  rename from gamma-new
  |  rename to gamma
  |
  o  7 24a0764120d700b1701941eb3f67454157136836 rename and add submodule
  |  branch=default convert_revision=59fb8e82ea18f79eab99196f588e8948089c134f hg-git-rename-source=git
  |
  |  diff --git a/.gitmodules b/.gitmodules
  |  --- a/.gitmodules
  |  +++ b/.gitmodules
  |  @@ -1,3 +1,6 @@
  |   [submodule "gitsubmodule"]
  |   	path = gitsubmodule2
  |   	url = ../gitsubmodule
  |  +[submodule "gamma"]
  |  +	path = gamma
  |  +	url = ../gitsubmodule
  |  diff --git a/.hgsub b/.hgsub
  |  --- a/.hgsub
  |  +++ b/.hgsub
  |  @@ -1,1 +1,2 @@
  |   gitsubmodule2 = [git]../gitsubmodule
  |  +gamma = [git]../gitsubmodule
  |  diff --git a/.hgsubstate b/.hgsubstate
  |  --- a/.hgsubstate
  |  +++ b/.hgsubstate
  |  @@ -1,1 +1,2 @@
  |  +5944b31ff85b415573d1a43eb942e2dea30ab8be gamma
  |   5944b31ff85b415573d1a43eb942e2dea30ab8be gitsubmodule2
  |  diff --git a/gamma b/gamma-new
  |  rename from gamma
  |  rename to gamma-new
  |
  o  6 4dfc1ab4a0ecd9066e613d8728fc6347f910801d beta renamed back
  |  branch=default convert_revision=f95497455dfa891b4cd9b524007eb9514c3ab654 hg-git-rename-source=git
  |
  |  diff --git a/beta b/beta
  |  old mode 120000
  |  new mode 100644
  |  --- a/beta
  |  +++ b/beta
  |  @@ -1,1 +1,12 @@
  |  -beta-new
  |  \ No newline at end of file
  |  +1
  |  +2
  |  +3
  |  +4
  |  +5
  |  +6
  |  +7
  |  +8
  |  +9
  |  +10
  |  +11
  |  +12
  |  diff --git a/beta-new b/beta-new
  |  deleted file mode 100644
  |  --- a/beta-new
  |  +++ /dev/null
  |  @@ -1,12 +0,0 @@
  |  -1
  |  -2
  |  -3
  |  -4
  |  -5
  |  -6
  |  -7
  |  -8
  |  -9
  |  -10
  |  -11
  |  -12
  |
  o  5 4bc4be232b3d6e53ddef658ef3d9ab142b3d0a0f beta renamed
  |  branch=default convert_revision=055f482277da6cd3dd37c7093d06983bad68f782 hg-git-rename-source=git
  |
  |  diff --git a/beta b/beta
  |  old mode 100644
  |  new mode 120000
  |  --- a/beta
  |  +++ b/beta
  |  @@ -1,12 +1,1 @@
  |  -1
  |  -2
  |  -3
  |  -4
  |  -5
  |  -6
  |  -7
  |  -8
  |  -9
  |  -10
  |  -11
  |  -12
  |  +beta-new
  |  \ No newline at end of file
  |  diff --git a/beta b/beta-new
  |  copy from beta
  |  copy to beta-new
  |
  o  4 ffad978ae6259b23aae39d566ec89d3af85a6c7a move submodule
  |  branch=default convert_revision=d7f31298f27df8a9226eddb1e4feb96922c46fa5 hg-git-rename-source=git
  |
  |  diff --git a/.gitmodules b/.gitmodules
  |  --- a/.gitmodules
  |  +++ b/.gitmodules
  |  @@ -1,3 +1,3 @@
  |   [submodule "gitsubmodule"]
  |  -	path = gitsubmodule
  |  +	path = gitsubmodule2
  |   	url = ../gitsubmodule
  |  diff --git a/.hgsub b/.hgsub
  |  --- a/.hgsub
  |  +++ b/.hgsub
  |  @@ -1,1 +1,1 @@
  |  -gitsubmodule = [git]../gitsubmodule
  |  +gitsubmodule2 = [git]../gitsubmodule
  |  diff --git a/.hgsubstate b/.hgsubstate
  |  --- a/.hgsubstate
  |  +++ b/.hgsubstate
  |  @@ -1,1 +1,1 @@
  |  -5944b31ff85b415573d1a43eb942e2dea30ab8be gitsubmodule
  |  +5944b31ff85b415573d1a43eb942e2dea30ab8be gitsubmodule2
  |
  o  3 c8c8ef153360b7368535b6ca01b7e02a7998b67e add submodule
  |  branch=default convert_revision=c610256cb6959852d9e70d01902a06726317affc hg-git-rename-source=git
  |
  |  diff --git a/.gitmodules b/.gitmodules
  |  new file mode 100644
  |  --- /dev/null
  |  +++ b/.gitmodules
  |  @@ -0,0 +1,3 @@
  |  +[submodule "gitsubmodule"]
  |  +	path = gitsubmodule
  |  +	url = ../gitsubmodule
  |  diff --git a/.hgsub b/.hgsub
  |  new file mode 100644
  |  --- /dev/null
  |  +++ b/.hgsub
  |  @@ -0,0 +1,1 @@
  |  +gitsubmodule = [git]../gitsubmodule
  |  diff --git a/.hgsubstate b/.hgsubstate
  |  new file mode 100644
  |  --- /dev/null
  |  +++ b/.hgsubstate
  |  @@ -0,0 +1,1 @@
  |  +5944b31ff85b415573d1a43eb942e2dea30ab8be gitsubmodule
  |
  o  2 e3e6b2083b5cc4382f611b16d23df93a40a19a00 gamma
  |  branch=default convert_revision=e1348449e0c3a417b086ed60fc13f068d4aa8b26 hg-git-rename-source=git
  |
  |  diff --git a/beta b/beta
  |  --- a/beta
  |  +++ b/beta
  |  @@ -9,3 +9,4 @@
  |   9
  |   10
  |   11
  |  +12
  |  diff --git a/beta b/gamma
  |  copy from beta
  |  copy to gamma
  |  --- a/beta
  |  +++ b/gamma
  |  @@ -9,3 +9,4 @@
  |   9
  |   10
  |   11
  |  +13
  |
  o  1 80be639891f44172f321d555badcbc3f9d11fa87 beta
  |  branch=default convert_revision=cc83241f39927232f690d370894960b0d1943a0e hg-git-rename-source=git
  |
  |  diff --git a/alpha b/beta
  |  rename from alpha
  |  rename to beta
  |  --- a/alpha
  |  +++ b/beta
  |  @@ -8,3 +8,4 @@
  |   8
  |   9
  |   10
  |  +11
  |
  o  0 ff861f77355d7a6aba082ff95f2bc716cf192980 alpha
     branch=default convert_revision=938bb65bb322eb4a3558bec4cdc8a680c4d1794c hg-git-rename-source=git
  
     diff --git a/alpha b/alpha
     new file mode 100644
     --- /dev/null
     +++ b/alpha
     @@ -0,0 +1,10 @@
     +1
     +2
     +3
     +4
     +5
     +6
     +7
     +8
     +9
     +10
  

Make a new ordinary commit in Mercurial (no extra metadata)
  $ echo 14 >> gamma
  $ hg ci -m "gamma2"

Make a new commit with a copy and a rename in Mercurial
  $ hg cp gamma delta
  $ echo 15 >> delta
  $ hg mv beta epsilon
  $ echo 16 >> epsilon
  $ hg ci -m "delta/epsilon"
  $ hg export .
  # HG changeset patch
  # User test
  # Date 0 0
  #      Thu Jan 01 00:00:00 1970 +0000
  # Node ID e01c1320d15c0a7abe4d30f3f71e03b2058a002f
  # Parent  3c7e506adb7b7a2b82f887d2923ee4d955695ca2
  delta/epsilon
  
  diff --git a/gamma b/delta
  copy from gamma
  copy to delta
  --- a/gamma
  +++ b/delta
  @@ -11,3 +11,4 @@
   11
   13
   14
  +15
  diff --git a/beta b/epsilon
  rename from beta
  rename to epsilon
  --- a/beta
  +++ b/epsilon
  @@ -10,3 +10,4 @@
   10
   11
   12
  +16
  $ hg push
  pushing to $TESTTMP/gitrepo
  searching for changes
  adding objects
  added 2 commits with 2 trees and 3 blobs
  updating reference refs/heads/master

  $ cd ../gitrepo
  $ git log master --pretty=oneline
  5f2948d029693346043f320620af99a615930dc4 delta/epsilon
  bbd2ec050f7fbc64f772009844f7d58a556ec036 gamma2
  50d116676a308b7c22935137d944e725d2296f2a remove submodule and rename back
  59fb8e82ea18f79eab99196f588e8948089c134f rename and add submodule
  f95497455dfa891b4cd9b524007eb9514c3ab654 beta renamed back
  055f482277da6cd3dd37c7093d06983bad68f782 beta renamed
  d7f31298f27df8a9226eddb1e4feb96922c46fa5 move submodule
  c610256cb6959852d9e70d01902a06726317affc add submodule
  e1348449e0c3a417b086ed60fc13f068d4aa8b26 gamma
  cc83241f39927232f690d370894960b0d1943a0e beta
  938bb65bb322eb4a3558bec4cdc8a680c4d1794c alpha

Make sure the right metadata is stored
  $ git cat-file commit "master^"
  tree 0adbde18545845f3b42ad1a18939ed60a9dec7a8
  parent 50d116676a308b7c22935137d944e725d2296f2a
  author test <none@none> 0 +0000
  committer test <none@none> 0 +0000
  HG:rename-source hg
  
  gamma2
  $ git cat-file commit master
  tree f8f32f4e20b56a5a74582c6a5952c175bf9ec155
  parent bbd2ec050f7fbc64f772009844f7d58a556ec036
  author test <none@none> 0 +0000
  committer test <none@none> 0 +0000
  HG:rename gamma:delta
  HG:rename beta:epsilon
  
  delta/epsilon

Now make another clone and compare the hashes

  $ cd ..
  $ hg clone -q gitrepo hgrepo2
  $ cd hgrepo2
  $ hg book master -qf
  $ hg export master
  # HG changeset patch
  # User test
  # Date 0 0
  #      Thu Jan 01 00:00:00 1970 +0000
  # Node ID 5ba38aef569229c46e403f4ab17c16520e40108e
  # Parent  2423517f443d79f55da435dfc95b828bf7b56d94
  delta/epsilon
  
  diff --git a/gamma b/delta
  copy from gamma
  copy to delta
  --- a/gamma
  +++ b/delta
  @@ -11,3 +11,4 @@
   11
   13
   14
  +15
  diff --git a/beta b/epsilon
  rename from beta
  rename to epsilon
  --- a/beta
  +++ b/epsilon
  @@ -10,3 +10,4 @@
   10
   11
   12
  +16

Regenerate the Git metadata and compare the hashes
  $ hg gclear
  clearing out the git cache data
  $ hg gexport
  $ cd .hg/git
  $ git log master --pretty=oneline
  9a60acf6a3a7cfaa723628d9f611f984ffdbe223 delta/epsilon
  48cc0b1b9c2aa12caa72a86fd5d9a6dbbf392352 gamma2
  44dbe9ba6dab7d29c312cd7001e9f1b5c52c612d remove submodule and rename back
  fb9c13c5ac5994b47906f59376e999cbb7017327 rename and add submodule
  dcdc3b83eec5f3b214e1cb5199edecf72d60de58 beta renamed back
  e805d81c8372a4d0d6a22ec5abb35e3467043588 beta renamed
  4899220730aca75da3f8f311f40bba30a7bd42ca move submodule
  843e2dc46308ace4649dbc9c82ff87f2cf420e09 add submodule
  00d2f34d2f9e2230ed49343b2b3eb14637b16c2e gamma
  e3ddc25bf9b3a6a00e4379673ee2d9d6bbef720e beta
  8557a753ca442f07736d74570a9cfebde4bf02e9 alpha

Test findcopiesharder

  $ cd $TESTTMP
  $ git init -q gitcopyharder
  $ cd gitcopyharder
  $ cat >> file0 << EOF
  > 1
  > 2
  > 3
  > 4
  > 5
  > EOF
  $ git add file0
  $ fn_git_commit -m file0
  $ cp file0 file1
  $ git add file1
  $ fn_git_commit -m file1
  $ cp file0 file2
  $ echo 6 >> file2
  $ git add file2
  $ fn_git_commit -m file2

  $ cd ..

Clone without findcopiesharder does not find copies from unmodified files

  $ hg clone gitcopyharder hgnocopyharder
  importing git objects into hg
  updating to branch default
  3 files updated, 0 files merged, 0 files removed, 0 files unresolved
  $ hg -R hgnocopyharder export 1::2
  # HG changeset patch
  # User test <test@example.org>
  # Date 1167609621 0
  #      Mon Jan 01 00:00:21 2007 +0000
  # Node ID 2e1fd38583278f0b6ede71d0f913f02ff3e14a36
  # Parent  3557dd9e8accc08148642bb4d8b2a4028e85f1f9
  file1
  
  diff --git a/file1 b/file1
  new file mode 100644
  --- /dev/null
  +++ b/file1
  @@ -0,0 +1,5 @@
  +1
  +2
  +3
  +4
  +5
  # HG changeset patch
  # User test <test@example.org>
  # Date 1167609622 0
  #      Mon Jan 01 00:00:22 2007 +0000
  # Node ID 6b935af41daea1bf80d299ea139fab32c937e2b0
  # Parent  2e1fd38583278f0b6ede71d0f913f02ff3e14a36
  file2
  
  diff --git a/file2 b/file2
  new file mode 100644
  --- /dev/null
  +++ b/file2
  @@ -0,0 +1,6 @@
  +1
  +2
  +3
  +4
  +5
  +6

findcopiesharder finds copies from unmodified files if similarity is met

  $ hg --config git.findcopiesharder=true clone gitcopyharder hgcopyharder0
  importing git objects into hg
  updating to branch default
  3 files updated, 0 files merged, 0 files removed, 0 files unresolved
  $ hg -R hgcopyharder0 export 1::2
  # HG changeset patch
  # User test <test@example.org>
  # Date 1167609621 0
  #      Mon Jan 01 00:00:21 2007 +0000
  # Node ID 822f61b91c7d74a67114314c5f6b078d9de4f3ac
  # Parent  3557dd9e8accc08148642bb4d8b2a4028e85f1f9
  file1
  
  diff --git a/file0 b/file1
  copy from file0
  copy to file1
  # HG changeset patch
  # User test <test@example.org>
  # Date 1167609622 0
  #      Mon Jan 01 00:00:22 2007 +0000
  # Node ID 6827e4ffec1b7f3e8b0e96a995adcc5fce4f8e8b
  # Parent  822f61b91c7d74a67114314c5f6b078d9de4f3ac
  file2
  
  diff --git a/file0 b/file2
  copy from file0
  copy to file2
  --- a/file0
  +++ b/file2
  @@ -3,3 +3,4 @@
   3
   4
   5
  +6

  $ hg --config git.findcopiesharder=true --config git.similarity=95 clone gitcopyharder hgcopyharder1
  importing git objects into hg
  updating to branch default
  3 files updated, 0 files merged, 0 files removed, 0 files unresolved
  $ hg -R hgcopyharder1 export 1::2
  # HG changeset patch
  # User test <test@example.org>
  # Date 1167609621 0
  #      Mon Jan 01 00:00:21 2007 +0000
  # Node ID 822f61b91c7d74a67114314c5f6b078d9de4f3ac
  # Parent  3557dd9e8accc08148642bb4d8b2a4028e85f1f9
  file1
  
  diff --git a/file0 b/file1
  copy from file0
  copy to file1
  # HG changeset patch
  # User test <test@example.org>
  # Date 1167609622 0
  #      Mon Jan 01 00:00:22 2007 +0000
  # Node ID 481884f836b7ce63906ad6875ac53b5bc5df134c
  # Parent  822f61b91c7d74a67114314c5f6b078d9de4f3ac
  file2
  
  diff --git a/file2 b/file2
  new file mode 100644
  --- /dev/null
  +++ b/file2
  @@ -0,0 +1,6 @@
  +1
  +2
  +3
  +4
  +5
  +6

Config values out of range
  $ hg --config git.similarity=999 clone gitcopyharder hgcopyharder2
  importing git objects into hg
  abort: git.similarity must be between 0 and 100
  [255]
  $ hg --config git.renamelimit=-5 clone gitcopyharder hgcopyharder2
  importing git objects into hg
  abort: git.renamelimit must be non-negative
  [255]
