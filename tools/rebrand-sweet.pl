#!/usr/bin/perl
# sweet -> bergamota across a git checkout (contents, then tracked paths). Run from the repo root:
#   perl tools/rebrand-sweet.pl [--dry]
# Kept as is: lines naming the upstream project (ios-sweet: credits, its repos, its author's disk paths),
# the Redmi Note 10 Pro's codename (bare word "sweet", "xiaomi,sweet", BOARD=sweet / --board sweet), and
# local build artifacts named in Markdown (sweet-0057hi.adt, ...img) that stay on disk under the old name.
# Deterministic per line, so it can be re-run on other branches before merging them.
use strict; use warnings;
my $dry = grep { $_ eq '--dry' } @ARGV;
my %case = (sweet => 'bergamota', Sweet => 'Bergamota', SWEET => 'BERGAMOTA');

sub swap_line {
    my ($l, $md) = @_;
    return $l if $l =~ /ios-sweet/i;
    my @keep;
    my $hold = sub { push @keep, $_[0]; "\x00" . $#keep . "\x00" };
    $l =~ s/(xiaomi,sweet)/$hold->($1)/ge;
    $l =~ s/([\w\/.-]*sweet[\w.-]*\.(?:img|simg|adt|txt|log|hfs|bin|tar|zip|png|jpg|mp4))/$hold->($1)/ge if $md;
    $l =~ s/((?<![A-Za-z0-9_.\/-])sweet(?![A-Za-z0-9_.\/-]))/$hold->($1)/ge;
    $l =~ s/(sweet|Sweet|SWEET)/$case{$1}/g;
    $l =~ s/\x00(\d+)\x00/$keep[$1]/g;
    return $l;
}

my @files = split /\0/, `git grep -z -l -I -i sweet`;
@files = grep { !/rebrand-sweet\.pl$/ } @files;
my ($nf, $nl) = (0, 0);
for my $f (@files) {
    next if -l $f;
    open my $in, '<:raw', $f or die "$f: $!";
    my @lines = <$in>; close $in;
    my $md = $f =~ /\.md$/i;
    my $changed = 0;
    for (@lines) { my $n = swap_line($_, $md); if ($n ne $_) { $_ = $n; $changed++ } }
    next unless $changed;
    $nf++; $nl += $changed;
    next if $dry;
    open my $out, '>:raw', $f or die "$f: $!"; print $out @lines; close $out;
}
print "contents: $nl lines in $nf files\n";

# Paths: rename the deepest components first so parents still exist when children move.
my @paths = grep { /sweet/i && !/rebrand-sweet\.pl$/ } split /\0/, `git ls-files -z`;
my %dirs;
for my $p (@paths) {
    my @c = split m{/}, $p;
    for my $i (0 .. $#c) { $dirs{join '/', @c[0 .. $i]} = $i if $c[$i] =~ /sweet/i }
}
my $np = 0;
for my $p (sort { $dirs{$b} <=> $dirs{$a} or $b cmp $a } keys %dirs) {
    my @c = split m{/}, $p;
    my $leaf = pop @c;
    (my $new = $leaf) =~ s/(sweet|Sweet|SWEET)/$case{$1}/g;
    next if $new eq $leaf;
    my $to = join '/', @c, $new;
    $np++;
    print "$p -> $to\n" if $dry;
    next if $dry;
    system('git', 'mv', $p, $to) == 0 or die "git mv $p failed";
}
print "paths: $np renamed\n";

# self-check of the line rules
my %t = ('sweet_drm_info(x)' => 'bergamota_drm_info(x)', 'SweetSurface' => 'BergamotaSurface',
         'SWEET FB:' => 'BERGAMOTA FB:', 'board sweet' => 'board sweet', 'xiaomi,sweet' => 'xiaomi,sweet',
         'see ios-sweet sweet_x' => 'see ios-sweet sweet_x', 'org.sweet.dropbear' => 'org.bergamota.dropbear',
         'kern.sweet_gpu' => 'kern.bergamota_gpu', "'sweet'" => "'sweet'", 'BOARD=sweet' => 'BOARD=sweet');
for (sort keys %t) { my $g = swap_line($_, 0); die "self-check: '$_' -> '$g', want '$t{$_}'\n" if $g ne $t{$_} }
die "self-check md\n" if swap_line("shim/out/sweet-0057hi.adt sweet_x", 1) ne "shim/out/sweet-0057hi.adt bergamota_x";
